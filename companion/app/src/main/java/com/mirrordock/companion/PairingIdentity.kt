package com.mirrordock.companion

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.security.KeyPair
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.Signature
import java.security.interfaces.ECPublicKey
import java.security.spec.ECGenParameterSpec

/**
 * 伴侣端长期身份（M4-1 配对产品化）。
 *
 * - EC P-256 密钥对生成于 Android KeyStore，私钥**不可导出**（非对称硬件背书，
 *   清除应用数据即销毁——与「解除互信」语义一致）。
 * - 公钥以 SPKI（X.509 SubjectPublicKeyInfo）DER 的 hex 上报给桌面端，
 *   与桌面端指纹计算（SHA-256(SPKI)）同源。
 * - 签名算法 SHA256withECDSA（输出 DER 编码），桌面端用 p256 验证。
 * - 首配时由桌面端发起 nonce 挑战，本类完成签名——证明对端持有申报的私钥。
 */
object PairingIdentity {

    private const val KEYSTORE = "AndroidKeyStore"
    private const val ALIAS = "mirrordock_companion_identity"

    /** 取（或首次生成）身份公钥的 SPKI DER hex。阻塞式，须在后台线程调用。 */
    fun publicKeySpkiHex(): String {
        val publicKey = ensureKeyPair().public as ECPublicKey
        // Java 的 publicKey.encoded 即 X.509 SPKI DER，与桌面端指纹计算同源。
        return publicKey.encoded.joinToString("") { "%02x".format(it) }
    }

    /** 用身份私钥对 data 做 SHA256withECDSA 签名，返回 DER 编码签名。 */
    fun sign(data: ByteArray): ByteArray {
        val entry = privateKeyEntry()
        val signature = Signature.getInstance("SHA256withECDSA")
        signature.initSign(entry.privateKey)
        signature.update(data)
        return signature.sign()
    }

    private fun ensureKeyPair(): KeyPair {
        val keyStore = KeyStore.getInstance(KEYSTORE).apply { load(null) }
        val existing = keyStore.getEntry(ALIAS, null) as? KeyStore.PrivateKeyEntry
        if (existing != null) {
            val publicKey = existing.certificate.publicKey as ECPublicKey
            return KeyPair(publicKey, existing.privateKey)
        }
        val generator = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, KEYSTORE)
        val spec = KeyGenParameterSpec.Builder(
            ALIAS,
            KeyProperties.PURPOSE_SIGN or KeyProperties.PURPOSE_VERIFY,
        )
            .setAlgorithmParameterSpec(ECGenParameterSpec("secp256r1"))
            .setDigests(KeyProperties.DIGEST_SHA256)
            .build()
        generator.initialize(spec)
        return generator.generateKeyPair()
    }

    private fun privateKeyEntry(): KeyStore.PrivateKeyEntry {
        val keyStore = KeyStore.getInstance(KEYSTORE).apply { load(null) }
        return keyStore.getEntry(ALIAS, null) as KeyStore.PrivateKeyEntry
    }
}
