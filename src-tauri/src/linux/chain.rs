//! Chaînes de filtres PipeWire — une par application configurée.
//!
//! Chaque chaîne est un petit processus `pipewire -c <conf>` qui charge
//! `libpipewire-module-filter-chain` et expose :
//! - un **sink virtuel** `mixflow.<slug>` (là où l'appli joue),
//! - un flux de lecture `mixflow.<slug>.out` relié à la sortie choisie.
//!
//! Le graphe est figé : 10 biquads `bq_peaking` puis un nœud `linear`
//! (volume / mute). Les points d'EQ inutilisés restent à 0 dB, donc
//! transparents : ajouter ou retirer un point, déplacer une fréquence ou
//! bouger le volume se fait **à chaud** via `pw-cli set-param … Props`,
//! sans redémarrer la chaîne ni couper le son.
//!
//! Les chaînes tournent comme services systemd **utilisateur**
//! (`mixflow-chain@<slug>.service`, rattachés à `pipewire.service`) : elles
//! démarrent avec la session et continuent de traiter le son quand la
//! fenêtre MixFlow est fermée. Aucun droit root nécessaire.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::model::{AppSettings, EqBand, MAX_BANDS};

/// Q des filtres peaking — identique à `EQ_Q` de `src/eqMath.ts`.
pub const EQ_Q: f32 = 1.0;
const UNIT_NAME: &str = "mixflow-chain@.service";

pub fn sink_name(slug: &str) -> String {
    format!("mixflow.{slug}")
}

pub fn playback_name(slug: &str) -> String {
    format!("mixflow.{slug}.out")
}

pub fn unit_instance(slug: &str) -> String {
    format!("mixflow-chain@{slug}.service")
}

/// Chaîne JSON SPA sûre (guillemets et antislashs neutralisés).
fn spa_str(s: &str) -> String {
    let clean: String = s
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| if c == '"' || c == '\\' { '\'' } else { c })
        .collect();
    format!("\"{clean}\"")
}

/// Les 10 points effectifs : ceux de l'utilisateur, complétés par des
/// points neutres (0 dB) répartis sur le spectre.
fn padded_bands(bands: &[EqBand]) -> Vec<EqBand> {
    let mut out: Vec<EqBand> = bands.iter().take(MAX_BANDS).cloned().collect();
    let mut i = 0;
    while out.len() < MAX_BANDS {
        out.push(EqBand {
            freq: 1000.0 + i as f32,
            gain: 0.0,
        });
        i += 1;
    }
    out
}

/// Valeur appliquée au nœud `linear` (Mult).
pub fn effective_volume(app: &AppSettings) -> f32 {
    if app.muted {
        0.0
    } else {
        app.gain
    }
}

/// Toutes les commandes de contrôle de la chaîne, dans l'ordre.
pub fn controls(app: &AppSettings) -> Vec<(String, f32)> {
    let mut c = Vec::with_capacity(MAX_BANDS * 3 + 1);
    for (i, b) in padded_bands(&app.eq_bands).iter().enumerate() {
        c.push((format!("eq{i}:Freq"), b.freq));
        c.push((format!("eq{i}:Q"), EQ_Q));
        c.push((format!("eq{i}:Gain"), b.gain));
    }
    c.push(("vol:Mult".into(), effective_volume(app)));
    c
}

/// Pod `Props` pour `pw-cli set-param <id> Props '<ici>'`.
pub fn props_pod(app: &AppSettings) -> String {
    let mut s = String::from("{ params = [");
    for (k, v) in controls(app) {
        let _ = write!(s, " \"{k}\" {v:.4}");
    }
    s.push_str(" ] }");
    s
}

/// Fichier de configuration PipeWire complet de la chaîne d'une appli.
pub fn render_conf(app: &AppSettings) -> String {
    let mut nodes = String::new();
    for (i, b) in padded_bands(&app.eq_bands).iter().enumerate() {
        let _ = writeln!(
            nodes,
            "          {{ type = builtin name = eq{i} label = bq_peaking control = {{ \"Freq\" = {:.2} \"Q\" = {EQ_Q:.2} \"Gain\" = {:.2} }} }}",
            b.freq, b.gain
        );
    }
    let _ = writeln!(
        nodes,
        "          {{ type = builtin name = vol label = linear control = {{ \"Mult\" = {:.4} \"Add\" = 0.0 }} }}",
        effective_volume(app)
    );
    let mut links = String::new();
    for i in 0..MAX_BANDS {
        let next = if i + 1 < MAX_BANDS {
            format!("eq{}", i + 1)
        } else {
            "vol".into()
        };
        let _ = writeln!(
            links,
            "          {{ output = \"eq{i}:Out\" input = \"{next}:In\" }}"
        );
    }
    let target = if app.output.is_empty() {
        String::new()
    } else {
        format!("\n        target.object = {}", spa_str(&app.output))
    };
    let slug = &app.slug;
    let desc = spa_str(&format!("MixFlow · {}", app.name));
    let desc_out = spa_str(&format!("MixFlow · {} (sortie)", app.name));
    let key = spa_str(&app.key);
    format!(
        r#"# Généré par MixFlow — ne pas modifier à la main (réécrit à chaque réglage).
# Appli : {name}
context.properties = {{
    log.level = 0
}}
context.spa-libs = {{
    audio.convert.* = audioconvert/libspa-audioconvert
    support.*       = support/libspa-support
}}
context.modules = [
    {{ name = libpipewire-module-rt flags = [ ifexists nofail ] }}
    {{ name = libpipewire-module-protocol-native }}
    {{ name = libpipewire-module-client-node }}
    {{ name = libpipewire-module-adapter }}
    {{ name = libpipewire-module-filter-chain
      args = {{
        node.description = {desc}
        media.name       = {desc}
        filter.graph = {{
          nodes = [
{nodes}          ]
          links = [
{links}          ]
        }}
        audio.channels = 2
        audio.position = [ FL FR ]
        capture.props = {{
          node.name        = "{sink}"
          node.description = {desc}
          media.class      = Audio/Sink
          # Jamais choisi comme sortie par défaut du système.
          priority.session = 1
          priority.driver  = 1
          mixflow.app      = {key}
        }}
        playback.props = {{
          node.name        = "{out}"
          node.description = {desc_out}
          node.passive     = true
          application.name = "MixFlow {slug}"
          # La cible est pilotée par MixFlow, pas par la mémoire de WirePlumber.
          state.restore-target = false
          mixflow.app      = {key}{target}
        }}
      }}
    }}
]
"#,
        name = spa_str(&app.name),
        sink = sink_name(slug),
        out = playback_name(slug),
    )
}

/// Unité systemd modèle, une instance par appli.
fn render_unit(chains_dir: &Path, pipewire_bin: &str) -> String {
    format!(
        "# Généré par MixFlow.\n\
[Unit]\n\
Description=MixFlow — chaîne audio %i (volume + EQ)\n\
After=pipewire.service wireplumber.service\n\
BindsTo=pipewire.service\n\
\n\
[Service]\n\
Type=simple\n\
ExecStart={pipewire_bin} -c {dir}/%i.conf\n\
Restart=on-failure\n\
RestartSec=1\n\
Slice=session.slice\n\
\n\
[Install]\n\
WantedBy=pipewire.service\n",
        dir = chains_dir.display()
    )
}

fn systemctl(args: &[&str]) -> Result<(), String> {
    let out = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()
        .map_err(|e| format!("systemctl introuvable : {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "systemctl --user {} : {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Écrit un fichier seulement si son contenu change (tmp + rename).
fn write_if_changed(path: &Path, content: &str) -> Result<bool, String> {
    if std::fs::read_to_string(path).ok().as_deref() == Some(content) {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, content).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(true)
}

pub struct Chains {
    pub chains_dir: PathBuf,
    unit_dir: PathBuf,
}

impl Chains {
    pub fn new(config_dir: &Path) -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let unit_dir = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("systemd/user");
        Self {
            chains_dir: config_dir.join("chains"),
            unit_dir,
        }
    }

    fn conf_path(&self, slug: &str) -> PathBuf {
        self.chains_dir.join(format!("{slug}.conf"))
    }

    /// Installe/actualise l'unité modèle (daemon-reload si elle change).
    pub fn ensure_unit(&self) -> Result<(), String> {
        let bin = ["/usr/bin/pipewire", "/bin/pipewire"]
            .into_iter()
            .find(|p| Path::new(p).exists())
            .unwrap_or("pipewire");
        let changed = write_if_changed(
            &self.unit_dir.join(UNIT_NAME),
            &render_unit(&self.chains_dir, bin),
        )?;
        if changed {
            systemctl(&["daemon-reload"])?;
        }
        Ok(())
    }

    /// Réécrit la conf d'une appli (persistance des réglages pour le
    /// prochain démarrage de la chaîne). Ne redémarre rien.
    pub fn write_conf(&self, app: &AppSettings) -> Result<bool, String> {
        write_if_changed(&self.conf_path(&app.slug), &render_conf(app))
    }

    /// Crée/active la chaîne d'une appli (idempotent).
    pub fn start(&self, app: &AppSettings) -> Result<(), String> {
        self.write_conf(app)?;
        systemctl(&["enable", "--now", &unit_instance(&app.slug)])
    }

    /// Arrête et supprime définitivement la chaîne d'une appli.
    pub fn remove(&self, slug: &str) -> Result<(), String> {
        let r = systemctl(&["disable", "--now", &unit_instance(slug)]);
        let _ = std::fs::remove_file(self.conf_path(slug));
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::model::LinuxConfig;

    fn app() -> AppSettings {
        let mut c = LinuxConfig::default();
        c.add_app("firefox-esr", "Firefox \"ESR\"");
        c.apps.remove(0)
    }

    #[test]
    fn conf_has_fixed_graph_and_escaped_names() {
        let mut a = app();
        a.output = "alsa_output.hdmi".into();
        let conf = render_conf(&a);
        assert_eq!(conf.matches("label = bq_peaking").count(), MAX_BANDS);
        assert!(conf.contains("node.name        = \"mixflow.firefox-esr\""));
        assert!(conf.contains("target.object = \"alsa_output.hdmi\""));
        assert!(conf.contains("MixFlow · Firefox 'ESR'"));
        assert!(!conf.contains("\"ESR\""));
    }

    #[test]
    fn default_output_has_no_target() {
        assert!(!render_conf(&app()).contains("target.object"));
    }

    #[test]
    fn mute_zeroes_volume() {
        let mut a = app();
        a.gain = 0.8;
        a.muted = true;
        let pod = props_pod(&a);
        assert!(pod.contains("\"vol:Mult\" 0.0000"));
        assert_eq!(controls(&a).len(), MAX_BANDS * 3 + 1);
    }
}
