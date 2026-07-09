interface CheckboxProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label?: string;
  disabled?: boolean;
}

export const Checkbox = ({ checked, onChange, label, disabled = false }: CheckboxProps) => {
  const checkbox = (
    <button
      type="button"
      role="checkbox"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => !disabled && onChange(!checked)}
      className={`w-6 h-6 rounded flex items-center justify-center border-2 transition-all ${
        checked
          ? 'bg-blue-600 border-blue-600'
          : 'bg-gray-700 border-gray-500 hover:border-gray-400'
      } ${disabled ? '' : 'active:scale-95'}`}
    >
      {checked && (
        <svg
          className="w-4 h-4 text-white"
          fill="none"
          viewBox="0 0 24 24"
          stroke="currentColor"
          strokeWidth={3}
        >
          <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
        </svg>
      )}
    </button>
  );

  if (!label) {
    return checkbox;
  }

  return (
    <label
      className={`flex items-center gap-2 select-none ${
        disabled ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer'
      }`}
    >
      {checkbox}
      <span className="text-sm">{label}</span>
    </label>
  );
}
