// 让 expect 拥有 toBeInTheDocument 等 DOM 断言（vitest 版本入口）。
import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

// 每个测试后卸载组件，避免上一个测试的 DOM 泄漏进下一个测试。
afterEach(() => {
  cleanup();
});
