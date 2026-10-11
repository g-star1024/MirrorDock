// WASM 软解兜底（tinyh264 / h264bsd）。
//
// 为什么需要它：WebCodecs 只在「安全上下文」（HTTPS 或 localhost）暴露，
// 而 MirrorDock NAS 的典型用法就是 http://局域网IP:8080 —— 非安全上下文，
// 浏览器直接没有 VideoDecoder。这条路径用 WASM 在 CPU 上解 H.264，
// 让局域网 HTTP 下也能看画面（代价是 CPU 占用更高、只吃 baseline）。
//
// 依赖：./vendor/h264bsd.mjs + ./vendor/h264bsd.wasm（Apache-2.0，已内置，离线可用）。
// 输入：服务端 video.data 已经是**剥掉 scrcpy 12 字节头的纯 Annex-B NAL**
//       （见 mirror_ws VideoDemux），config 帧内联 SPS/PPS——直接可喂 tinyh264。

import H264bsdModule from './vendor/h264bsd.mjs';

let modulePromise = null;

function loadModule() {
  if (!modulePromise) modulePromise = H264bsdModule();
  return modulePromise;
}

// YUV420 (I420) → RGBA，BT.601。tinyh264 输出 YUV 平面，Canvas 要 RGBA。
// 这是软解路径上除解码外最耗 CPU 的一步；查表 + 逐像素整数运算已是最朴素
// 可靠的写法。先求「能看」，再谈 WebGL 加速（局域网兜底场景帧率要求不高）。
const CLIP = (() => { const t = new Uint8Array(1024); for (let i = 0; i < 1024; i++) t[i] = i < 0 ? 0 : i > 255 ? 255 : i; return t; })();

function yuvToRgba(yBytes, uBytes, vBytes, w, h, out) {
  let di = 0;
  for (let row = 0; row < h; row++) {
    const yRow = row * w;
    const cRow = (row >> 1) * (w >> 1);
    for (let col = 0; col < w; col++) {
      const y = yBytes[yRow + col];
      const u = uBytes[cRow + (col >> 1)] - 128;
      const v = vBytes[cRow + (col >> 1)] - 128;
      // BT.601 整数近似
      let r = y + ((359 * v) >> 8);
      let g = y - ((88 * u + 183 * v) >> 8);
      let b = y + ((454 * u) >> 8);
      out[di++] = CLIP[r + 384];
      out[di++] = CLIP[g + 384];
      out[di++] = CLIP[b + 384];
      out[di++] = 255;
    }
  }
}

export class WasmDecoder {
  constructor() {
    this.decoder = null;
    this.picId = 0;
    this.configData = null;   // 缓存 SPS/PPS config 帧，分辨率变化后重喂
    this.sawConfig = false;
    this.width = 0;
    this.height = 0;
  }

  async init() {
    const mod = await loadModule();
    this.decoder = new mod.Decoder();
  }

  // 分辨率变化：tinyh264 没有 reset，直接弃旧建新，并重喂缓存的 SPS/PPS。
  async reset() {
    if (this.decoder) { try { this.decoder.delete(); } catch (_) {} }
    await this.init();
    this.sawConfig = false;
    if (this.configData) this._feed(this.configData);
  }

  _feed(data) {
    try {
      const res = this.decoder.decode(data, this.picId++);
      return res;
    } catch (_) {
      return null;
    }
  }

  // 喂一个 media 包（data 为 Uint8Array 纯 NAL；config/keyframe 为布尔）。
  // 成功渲染一帧时回调 onFrame(imageData, w, h)。
  decodePacket(data, config, keyframe, onFrame) {
    if (!this.decoder) return;
    if (config) { this.configData = data; this.sawConfig = true; }
    if (!this.sawConfig) return; // 还没拿到 SPS/PPS，解不了

    const res = this._feed(data);
    if (!res || !res.picture) return;

    this._emit(res.picture, onFrame);
    for (let i = 0; i < (res.extraPictureCount || 0); i++) {
      const p = this.decoder.getNextPicture();
      if (p) this._emit(p, onFrame);
    }
  }

  _emit(pic, onFrame) {
    const w = pic.info.cropParams.cropOutWidth || pic.info.picWidth;
    const h = pic.info.cropParams.cropOutHeight || pic.info.picHeight;
    const stride = pic.info.picWidth;
    const chromaW = stride >> 1;
    const uOffset = stride * pic.info.picHeight;
    const vOffset = uOffset + chromaW * (pic.info.picHeight >> 1);
    const yBytes = pic.bytes.subarray(0, uOffset);
    const uBytes = pic.bytes.subarray(uOffset, vOffset);
    const vBytes = pic.bytes.subarray(vOffset);

    // 输出尺寸 = 显示尺寸（crop 后）。YUV→RGB 用 crop 宽高逐行取，
    // stride 可能与 crop 宽不一致，逐行拷贝最稳。
    const rgba = new Uint8ClampedArray(w * h * 4);
    // 简化：当 stride == w 且无 crop 时走快路径，否则逐行。
    if (stride === w && pic.info.cropParams.cropOutWidth === w) {
      yuvToRgba(yBytes, uBytes, vBytes, w, h, rgba);
    } else {
      // 逐行重建紧凑 YUV 平面再转。
      const yP = new Uint8Array(w * h);
      const uP = new Uint8Array((w >> 1) * (h >> 1));
      const vP = new Uint8Array((w >> 1) * (h >> 1));
      const cl = pic.info.cropParams.cropLeftOffset, ct = pic.info.cropParams.cropTopOffset;
      for (let r = 0; r < h; r++) {
        yP.set(yBytes.subarray((ct + r) * stride + cl, (ct + r) * stride + cl + w), r * w);
      }
      const ch = h >> 1, cw = w >> 1;
      for (let r = 0; r < ch; r++) {
        uP.set(uBytes.subarray(((ct >> 1) + r) * chromaW + (cl >> 1), ((ct >> 1) + r) * chromaW + (cl >> 1) + cw), r * cw);
        vP.set(vBytes.subarray(((ct >> 1) + r) * chromaW + (cl >> 1), ((ct >> 1) + r) * chromaW + (cl >> 1) + cw), r * cw);
      }
      yuvToRgba(yP, uP, vP, w, h, rgba);
    }

    this.width = w; this.height = h;
    onFrame(new ImageData(rgba, w, h), w, h);
  }

  dispose() {
    if (this.decoder) { try { this.decoder.delete(); } catch (_) {} this.decoder = null; }
  }
}
