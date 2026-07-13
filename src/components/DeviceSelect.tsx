/**
 * Device dropdown, two flavors actually used in the app:
 * - role="mic": the Micro strip's source — physical capture devices only.
 * - role="speaker": any channel's "Sortie" — physical render devices only
 *   (virtual cables are hidden here to prevent feedback loops; a channel's
 *   own cable is auto-claimed and never chosen by the user).
 *
 * A selected device that vanished stays listed as "(manquant)" instead of
 * being silently dropped from the config.
 */

import { isVirtualDevice } from "../types";

interface Props {
  devices: string[];
  value: string;
  /** Text of the empty choice. */
  placeholder: string;
  /** Purely documents intent at call sites — both render identically. */
  role: "mic" | "speaker";
  onChange: (device: string) => void;
}

export default function DeviceSelect({ devices, value, placeholder, onChange }: Props) {
  const physical = devices.filter((d) => !isVirtualDevice(d));
  const missing = value !== "" && !devices.includes(value);

  return (
    <select
      className="device-select"
      value={value}
      onChange={(e) => onChange(e.target.value)}
      title={value || placeholder}
    >
      <option value="">{placeholder}</option>
      {physical.map((d) => (
        <option key={d} value={d}>
          {d}
        </option>
      ))}
      {missing && <option value={value}>{value} (manquant)</option>}
    </select>
  );
}
