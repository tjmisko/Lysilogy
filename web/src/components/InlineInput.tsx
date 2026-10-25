import { useRef, useState } from "react";

/** An input that saves on Enter and cancels on Esc or when it loses focus. */
export function InlineInput(props: {
  label: string;
  initial: string;
  className: string;
  maxLength?: number;
  placeholder?: string;
  onSave: (value: string) => void;
  onCancel: () => void;
}) {
  const { label, initial, className, maxLength = 2000, placeholder, onSave, onCancel } = props;
  const [value, setValue] = useState(initial);
  const finished = useRef(false);
  const finish = (save: boolean): void => {
    if (finished.current) return;
    finished.current = true;
    if (save) onSave(value);
    else onCancel();
  };
  return (
    <input
      className={className}
      aria-label={label}
      value={value}
      placeholder={placeholder}
      autoFocus
      maxLength={maxLength}
      onChange={(event) => setValue(event.target.value)}
      onBlur={() => finish(false)}
      onKeyDown={(event) => {
        if (event.nativeEvent.isComposing) return;
        if (event.key === "Enter") {
          event.preventDefault();
          event.stopPropagation();
          finish(true);
        } else if (event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          finish(false);
        }
      }}
    />
  );
}
