interface SliderProps {
  value: number;
  onChange: (value: number) => void;
  min?: number;
  max?: number;
  step?: number;
  label?: string;
  showValue?: boolean;
  disabled?: boolean;
}

export const Slider = ({
  value,
  onChange,
  min = 1,
  max = 10,
  step = 1,
  label,
  showValue = true,
  disabled = false,
}: SliderProps) => {
  const percentage = ((value - min) / (max - min)) * 100;

  return (
    <div className={`flex items-center gap-3 ${disabled ? 'opacity-50' : ''}`}>
      {label && <span className="text-sm text-gray-400 min-w-fit">{label}</span>}
      <div className="relative flex-1 flex items-center">
        <input
          type="range"
          min={min}
          max={max}
          step={step}
          value={value}
          onChange={(e) => !disabled && onChange(parseFloat(e.target.value))}
          disabled={disabled}
          className="w-full h-2 appearance-none cursor-pointer bg-transparent z-10 relative
            [&::-webkit-slider-thumb]:appearance-none
            [&::-webkit-slider-thumb]:w-5
            [&::-webkit-slider-thumb]:h-5
            [&::-webkit-slider-thumb]:rounded-full
            [&::-webkit-slider-thumb]:bg-blue-500
            [&::-webkit-slider-thumb]:border-2
            [&::-webkit-slider-thumb]:border-blue-400
            [&::-webkit-slider-thumb]:cursor-pointer
            [&::-webkit-slider-thumb]:transition-transform
            [&::-webkit-slider-thumb]:hover:scale-110
            [&::-webkit-slider-thumb]:active:scale-95
            [&::-moz-range-thumb]:w-5
            [&::-moz-range-thumb]:h-5
            [&::-moz-range-thumb]:rounded-full
            [&::-moz-range-thumb]:bg-blue-500
            [&::-moz-range-thumb]:border-2
            [&::-moz-range-thumb]:border-blue-400
            [&::-moz-range-thumb]:cursor-pointer"
        />
        {/* Track background */}
        <div className="absolute inset-0 flex items-center pointer-events-none">
          <div className="w-full h-2 rounded-full bg-gray-600">
            <div
              className="h-full rounded-full bg-blue-600 transition-all"
              style={{ width: `${percentage}%` }}
            />
          </div>
        </div>
      </div>
      {showValue && (
        <span className="text-sm font-medium min-w-[2ch] text-right">{value}</span>
      )}
    </div>
  );
}
