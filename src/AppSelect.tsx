import { useEffect, useRef, useState } from "react";

export type SelectOption = { value: string; label: string; hint?: string };

type AppSelectProps = {
  value: string;
  options: SelectOption[];
  onChange: (value: string) => void;
  ariaLabel?: string;
  disabled?: boolean;
  className?: string;
};

/**
 * 自绘下拉框（X10-86）：替换原生 <select>，统一客户端视觉语言。
 * 原生 select 在各平台渲染差异大（Windows 上是灰底黑框系统样式），与整体
 * 轻主题圆角设计不搭。这里用 button + 浮层列表自绘，交互与原生一致：
 * 点击展开、点选即合、Esc 关闭、点击外部关闭、↑↓ 移动高亮、Enter 选中。
 */
export function AppSelect({ value, options, onChange, ariaLabel, disabled, className }: AppSelectProps) {
  const [open, setOpen] = useState(false);
  const [highlight, setHighlight] = useState<number>(-1);
  const rootRef = useRef<HTMLDivElement>(null);

  const selectedIndex = options.findIndex((option) => option.value === value);
  const selected = selectedIndex >= 0 ? options[selectedIndex] : null;

  // 点击组件外部时收起。
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) {
        setOpen(false);
      }
    };
    window.addEventListener("pointerdown", onPointerDown);
    return () => window.removeEventListener("pointerdown", onPointerDown);
  }, [open]);

  // 展开时把高亮对齐到当前选中项。
  useEffect(() => {
    if (open) setHighlight(selectedIndex >= 0 ? selectedIndex : 0);
  }, [open, selectedIndex]);

  function commit(index: number) {
    const option = options[index];
    if (!option) return;
    onChange(option.value);
    setOpen(false);
  }

  function onKeyDown(event: React.KeyboardEvent) {
    if (disabled) return;
    if (!open) {
      if (event.key === "Enter" || event.key === " " || event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        setOpen(true);
      }
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      setOpen(false);
    } else if (event.key === "ArrowDown") {
      event.preventDefault();
      setHighlight((prev) => Math.min(prev + 1, options.length - 1));
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setHighlight((prev) => Math.max(prev - 1, 0));
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      commit(highlight);
    } else if (event.key === "Tab") {
      setOpen(false);
    }
  }

  return (
    <div className={`app-select${className ? ` ${className}` : ""}${open ? " open" : ""}${disabled ? " disabled" : ""}`} ref={rootRef}>
      <button
        type="button"
        className="app-select-trigger"
        aria-label={ariaLabel}
        aria-haspopup="listbox"
        aria-expanded={open}
        disabled={disabled}
        onClick={() => setOpen((prev) => !prev)}
        onKeyDown={onKeyDown}
      >
        <span className="app-select-value">{selected ? selected.label : "请选择"}</span>
        <svg className="app-select-caret" width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
          <path d="M3 4.5 6 7.5 9 4.5" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </button>
      {open && (
        <ul className="app-select-menu" role="listbox" aria-label={ariaLabel}>
          {options.map((option, index) => (
            <li key={option.value}>
              <button
                type="button"
                role="option"
                aria-selected={option.value === value}
                className={`app-select-option${index === highlight ? " highlight" : ""}${option.value === value ? " selected" : ""}`}
                onMouseEnter={() => setHighlight(index)}
                onClick={() => commit(index)}
              >
                <span className="app-select-option-label">{option.label}</span>
                {option.value === value && (
                  <svg className="app-select-check" width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
                    <path d="m3 7.4 2.6 2.6L11 4.4" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
                  </svg>
                )}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
