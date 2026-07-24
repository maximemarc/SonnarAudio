/**
 * Device dropdown, three flavors actually used in the app:
 * - role="mic": the Micro strip's source — physical capture devices only.
 * - role="speaker": any channel's "Sortie" — physical render devices only
 *   (virtual cables are hidden here to prevent feedback loops; a channel's
 *   own cable is auto-claimed and never chosen by the user).
 * - role="stream": le Mode Streamer, où l'on veut AU CONTRAIRE un câble
 *   virtuel (celui que capte OBS) — masquer les câbles y rendait la
 *   fonctionnalité littéralement inatteignable.
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
  /** "stream" garde les câbles virtuels ; les deux autres les masquent. */
  role: "mic" | "speaker" | "stream";
  onChange: (device: string) => void;
}

export default function DeviceSelect({ devices, value, placeholder, role, onChange }: Props) {
  const listed = role === "stream" ? devices : devices.filter((d) => !isVirtualDevice(d));
  const missing = value !== "" && !devices.includes(value);

  return (
    <select
      className="device-select"
      value={value}
      onChange={(e) => onChange(e.target.value)}
      title={value || placeholder}
    >
      <option value="">{placeholder}</option>
      {listed.map((d) => (
        <option key={d} value={d}>
          {d}
        </option>
      ))}
      {missing && <option value={value}>{value} (manquant)</option>}
    </select>
  );
}
