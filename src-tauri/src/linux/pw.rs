//! Vue du graphe PipeWire, alimentée par `pw-dump --monitor`.
//!
//! Pas de liaison à libpipewire (ni bindgen, ni paquet `-dev`) : on lit le
//! JSON de `pw-dump` (instantané puis mises à jour en continu) et on agit
//! via `pw-cli` et `pw-metadata`, les outils fournis avec PipeWire.

use std::collections::HashMap;
use std::io::BufReader;
use std::process::{Command, Stdio};

use serde_json::Value;

#[derive(Clone, Debug, Default)]
pub struct Node {
    pub id: u32,
    pub serial: String,
    pub name: String,
    pub description: String,
    pub media_class: String,
    pub app_name: String,
    pub binary: String,
    pub icon_name: String,
    pub state: String,
    /// `device.api` du périphérique (alsa, bluez5…), vide pour un flux.
    pub api: String,
    pub is_monitor_stream: bool,
}

impl Node {
    /// Clé d'identification d'appli (voir `model.rs`).
    pub fn app_key(&self) -> Option<String> {
        if !self.binary.is_empty() {
            Some(self.binary.clone())
        } else if !self.app_name.is_empty() {
            Some(self.app_name.clone())
        } else {
            None
        }
    }

    pub fn is_ours(&self) -> bool {
        self.name.starts_with("mixflow.")
    }

    /// Flux de lecture d'une appli (hors nos propres chaînes).
    pub fn is_app_playback(&self) -> bool {
        self.media_class == "Stream/Output/Audio" && !self.is_ours() && !self.is_monitor_stream
    }

    /// Sortie physique (ou Bluetooth) proposable à l'utilisateur.
    pub fn is_hw_sink(&self) -> bool {
        self.media_class == "Audio/Sink" && !self.is_ours()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Graph {
    pub nodes: HashMap<u32, Node>,
    /// Liens : id → (nœud de sortie, nœud d'entrée).
    pub links: HashMap<u32, (u32, u32)>,
    /// Entrées `target.object` de la métadonnée "default" : sujet → valeur.
    pub targets: HashMap<u32, String>,
    pub default_sink: String,
    pub default_metadata_id: Option<u32>,
    /// Vrai dès que le premier instantané a été lu.
    pub ready: bool,
}

fn s(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

impl Graph {
    pub fn node_by_name(&self, name: &str) -> Option<&Node> {
        self.nodes.values().find(|n| n.name == name)
    }

    /// Nœuds vers lesquels `id` est actuellement relié.
    #[cfg(test)]
    pub fn linked_to(&self, id: u32) -> impl Iterator<Item = u32> + '_ {
        self.links
            .values()
            .filter(move |(o, _)| *o == id)
            .map(|(_, i)| *i)
    }

    /// Applique un lot d'objets `pw-dump` (instantané ou mise à jour).
    pub fn apply(&mut self, batch: &[Value]) {
        for obj in batch {
            let Some(id) = obj.get("id").and_then(Value::as_u64).map(|x| x as u32) else {
                continue;
            };
            let info = obj.get("info");
            if info == Some(&Value::Null) {
                // Objet supprimé : `{"id": N, "info": null}` (le type n'est
                // pas rappelé, on purge partout). Attention : les objets
                // Metadata n'ont jamais de clé "info", ce n'est pas un retrait.
                self.nodes.remove(&id);
                self.links.remove(&id);
                if self.default_metadata_id == Some(id) {
                    self.default_metadata_id = None;
                    self.targets.clear();
                }
                continue;
            }
            let ty = obj.get("type").and_then(Value::as_str).unwrap_or("");
            match ty {
                "PipeWire:Interface:Node" => {
                    let Some(info) = info else { continue };
                    let p = info.get("props").cloned().unwrap_or(Value::Null);
                    let node = Node {
                        id,
                        serial: s(&p, "object.serial"),
                        name: s(&p, "node.name"),
                        description: {
                            let d = s(&p, "node.description");
                            if d.is_empty() {
                                s(&p, "node.nick")
                            } else {
                                d
                            }
                        },
                        media_class: s(&p, "media.class"),
                        app_name: s(&p, "application.name"),
                        binary: s(&p, "application.process.binary"),
                        icon_name: s(&p, "application.icon_name"),
                        state: s(info, "state"),
                        api: s(&p, "device.api"),
                        is_monitor_stream: s(&p, "stream.monitor") == "true"
                            || p.get("stream.capture.sink").is_some(),
                    };
                    self.nodes.insert(id, node);
                }
                "PipeWire:Interface:Link" => {
                    let Some(info) = info else { continue };
                    let o = info.get("output-node-id").and_then(Value::as_u64);
                    let i = info.get("input-node-id").and_then(Value::as_u64);
                    if let (Some(o), Some(i)) = (o, i) {
                        self.links.insert(id, (o as u32, i as u32));
                    }
                }
                "PipeWire:Interface:Metadata" => {
                    let name = obj
                        .get("props")
                        .map(|p| s(p, "metadata.name"))
                        .unwrap_or_default();
                    if name != "default" {
                        continue;
                    }
                    self.default_metadata_id = Some(id);
                    if let Some(entries) = obj.get("metadata").and_then(Value::as_array) {
                        self.targets.clear();
                        for e in entries {
                            let key = s(e, "key");
                            let subject = e.get("subject").and_then(Value::as_u64);
                            if key == "default.audio.sink" {
                                self.default_sink =
                                    e.get("value").map(|v| s(v, "name")).unwrap_or_default();
                            } else if key == "target.object" {
                                if let Some(sub) = subject {
                                    self.targets.insert(sub as u32, s(e, "value"));
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Lance `pw-dump --monitor` et appelle `on_batch` pour chaque lot JSON
/// (le premier est l'instantané complet). Rend la main quand le processus
/// se termine (PipeWire redémarré…) : l'appelant relance.
pub fn monitor(mut on_batch: impl FnMut(Vec<Value>)) -> Result<(), String> {
    let mut child = Command::new("pw-dump")
        .args(["--monitor", "--no-colors"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("pw-dump introuvable : {e}"))?;
    let stdout = child.stdout.take().ok_or("pw-dump sans sortie")?;
    let stream = serde_json::Deserializer::from_reader(BufReader::new(stdout)).into_iter::<Value>();
    for v in stream {
        match v {
            Ok(Value::Array(batch)) => on_batch(batch),
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}

fn run(cmd: &str, args: &[&str]) -> Result<(), String> {
    let out = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{cmd} : {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{cmd} {} : {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Envoie des paramètres de contrôle à un nœud filter-chain.
pub fn set_props(node_id: u32, pod: &str) -> Result<(), String> {
    run("pw-cli", &["set-param", &node_id.to_string(), "Props", pod])
}

/// Dirige un flux vers un nœud précis (`target_serial`), ou vers la sortie
/// par défaut (`None` → "-1"). WirePlumber relie aussitôt le flux.
pub fn set_target(stream_id: u32, target_serial: Option<&str>) -> Result<(), String> {
    let v = target_serial.unwrap_or("-1");
    // `--` : sinon "-1" serait lu comme une option.
    run(
        "pw-metadata",
        &["--", &stream_id.to_string(), "target.object", v, "Spa:Id"],
    )
}

/// Retire toute consigne de cible d'un flux (il redevient libre).
pub fn clear_target(stream_id: u32) -> Result<(), String> {
    run(
        "pw-metadata",
        &["-d", &stream_id.to_string(), "target.object"],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn graph_tracks_nodes_links_and_metadata() {
        let mut g = Graph::default();
        g.apply(&[
            json!({"id": 50, "type": "PipeWire:Interface:Node", "info": {"state": "running",
                "props": {"node.name": "alsa_output.hdmi", "media.class": "Audio/Sink",
                          "node.description": "HDMI", "object.serial": 77}}}),
            json!({"id": 90, "type": "PipeWire:Interface:Node", "info": {"state": "running",
                "props": {"node.name": "Firefox", "media.class": "Stream/Output/Audio",
                          "application.process.binary": "firefox-esr", "object.serial": 120}}}),
            json!({"id": 91, "type": "PipeWire:Interface:Link",
                "info": {"output-node-id": 90, "input-node-id": 50}}),
            json!({"id": 3, "type": "PipeWire:Interface:Metadata",
                "props": {"metadata.name": "default"},
                "metadata": [
                    {"subject": 0, "key": "default.audio.sink", "type": "Spa:String:JSON",
                     "value": {"name": "alsa_output.hdmi"}},
                    {"subject": 90, "key": "target.object", "type": "Spa:Id", "value": "77"}]}),
        ]);
        assert_eq!(g.default_sink, "alsa_output.hdmi");
        assert_eq!(g.targets.get(&90).map(String::as_str), Some("77"));
        let ff = &g.nodes[&90];
        assert!(ff.is_app_playback());
        assert_eq!(ff.app_key().as_deref(), Some("firefox-esr"));
        assert_eq!(g.nodes[&50].serial, "77");
        assert_eq!(g.linked_to(90).collect::<Vec<_>>(), vec![50]);
        g.apply(&[
            json!({"id": 90, "info": null}),
            json!({"id": 91, "info": null}),
        ]);
        assert!(!g.nodes.contains_key(&90));
        assert!(g.links.is_empty());
    }
}
