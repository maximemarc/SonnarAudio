//! Nom lisible et icône d'une appli, retrouvés via les fichiers `.desktop`
//! et le thème d'icônes de l'utilisateur (ex. WhiteSur-dark).
//!
//! L'icône est renvoyée en data-URI (SVG ou PNG) : la webview n'a pas accès
//! au système de fichiers. Tout est mis en cache — l'indexation des thèmes
//! ne se fait qu'une fois.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use base64::Engine as _;
use parking_lot::Mutex;

#[derive(Clone, Debug, Default)]
pub struct AppIdentity {
    pub name: Option<String>,
    pub icon: Option<String>,
}

struct DesktopEntry {
    name: String,
    icon: String,
    /// Identifiants comparés à la clé de l'appli (minuscules) : binaire de
    /// `Exec`, `StartupWMClass`, nom du fichier (et son dernier segment
    /// pour les ids inversés type `org.mozilla.firefox`).
    ids: Vec<String>,
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn data_dirs() -> Vec<PathBuf> {
    let mut v = vec![home().join(".local/share")];
    let sys =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    v.extend(sys.split(':').filter(|s| !s.is_empty()).map(PathBuf::from));
    v.push(PathBuf::from("/var/lib/flatpak/exports/share"));
    v.push(home().join(".local/share/flatpak/exports/share"));
    v
}

fn parse_desktop(path: &Path) -> Option<DesktopEntry> {
    let text = std::fs::read_to_string(path).ok()?;
    let (mut name, mut name_fr, mut icon, mut exec, mut wm) = (
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
    );
    let mut in_main = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        match k.trim() {
            "Name" => name = v.trim().into(),
            "Name[fr]" => name_fr = v.trim().into(),
            "Icon" => icon = v.trim().into(),
            "Exec" => exec = v.trim().into(),
            "StartupWMClass" => wm = v.trim().into(),
            _ => {}
        }
    }
    if name.is_empty() {
        return None;
    }
    let mut ids = Vec::new();
    // Premier mot de Exec qui n'est pas un lanceur (env, flatpak run…).
    for tok in exec.split_whitespace() {
        if tok.contains('=') || ["env", "flatpak", "run"].contains(&tok) || tok.starts_with('-') {
            continue;
        }
        if let Some(b) = Path::new(tok.trim_matches('"')).file_name() {
            ids.push(b.to_string_lossy().to_lowercase());
        }
        break;
    }
    if !wm.is_empty() {
        ids.push(wm.to_lowercase());
    }
    if let Some(stem) = path.file_stem() {
        let stem = stem.to_string_lossy().to_lowercase();
        if let Some(last) = stem.rsplit('.').next() {
            ids.push(last.to_string());
        }
        ids.push(stem);
    }
    Some(DesktopEntry {
        name: if name_fr.is_empty() { name } else { name_fr },
        icon,
        ids,
    })
}

fn desktop_entries() -> &'static Vec<DesktopEntry> {
    static ENTRIES: OnceLock<Vec<DesktopEntry>> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        let mut v = Vec::new();
        for d in data_dirs() {
            let Ok(rd) = std::fs::read_dir(d.join("applications")) else {
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "desktop") {
                    if let Some(de) = parse_desktop(&p) {
                        v.push(de);
                    }
                }
            }
        }
        v
    })
}

fn user_icon_theme() -> String {
    for (schema, key) in [
        ("org.mate.interface", "icon-theme"),
        ("org.gnome.desktop.interface", "icon-theme"),
    ] {
        if let Ok(out) = Command::new("gsettings")
            .args(["get", schema, key])
            .output()
        {
            let t = String::from_utf8_lossy(&out.stdout)
                .trim()
                .trim_matches('\'')
                .to_string();
            if out.status.success() && !t.is_empty() {
                return t;
            }
        }
    }
    "hicolor".into()
}

/// Index nom d'icône → meilleur fichier (SVG d'abord, puis le plus grand
/// PNG), pour le thème de l'utilisateur, ses parents usuels et hicolor.
fn icon_index() -> &'static HashMap<String, PathBuf> {
    static INDEX: OnceLock<HashMap<String, PathBuf>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let theme = user_icon_theme();
        let mut themes = vec![theme.clone()];
        if let Some(base) = theme
            .strip_suffix("-dark")
            .or_else(|| theme.strip_suffix("-light"))
        {
            themes.push(base.to_string());
        }
        themes.extend(["hicolor", "Adwaita"].map(String::from));
        let mut roots = vec![home().join(".icons")];
        roots.extend(data_dirs().into_iter().map(|d| d.join("icons")));

        let mut index: HashMap<String, (u32, PathBuf)> = HashMap::new();
        // Les thèmes sont parcourus du plus prioritaire au moins prioritaire :
        // un thème ne remplace jamais une icône déjà trouvée plus haut.
        for (rank, t) in themes.iter().enumerate() {
            let mut found: HashMap<String, (u32, PathBuf)> = HashMap::new();
            for r in &roots {
                walk_icons(&r.join(t), 0, &mut found);
            }
            for (k, (score, p)) in found {
                index
                    .entry(k)
                    .or_insert((score + (10 - rank as u32) * 100_000, p));
            }
        }
        // Dernier recours : /usr/share/pixmaps.
        if let Ok(rd) = std::fs::read_dir("/usr/share/pixmaps") {
            for e in rd.flatten() {
                let p = e.path();
                let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase());
                if matches!(ext.as_deref(), Some("png" | "svg")) {
                    if let Some(stem) = p.file_stem() {
                        index.entry(stem.to_string_lossy().into()).or_insert((0, p));
                    }
                }
            }
        }
        index.into_iter().map(|(k, (_, p))| (k, p)).collect()
    })
}

fn walk_icons(dir: &Path, depth: u32, found: &mut HashMap<String, (u32, PathBuf)>) {
    if depth > 4 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let dir_str = dir.to_string_lossy();
    // On ne garde que les icônes d'applis (pas les actions, statuts…).
    let in_apps = dir_str.contains("/apps");
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk_icons(&p, depth + 1, found);
            continue;
        }
        if !in_apps {
            continue;
        }
        let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase());
        let score = match ext.as_deref() {
            Some("svg") => 10_000,
            Some("png") => size_hint(&dir_str),
            _ => continue,
        };
        let Some(stem) = p.file_stem().map(|s| s.to_string_lossy().to_string()) else {
            continue;
        };
        let better = found.get(&stem).is_none_or(|(s, _)| score > *s);
        if better {
            found.insert(stem, (score, p));
        }
    }
}

/// Taille d'un dossier d'icônes PNG d'après son chemin (`48x48`, `256`…).
fn size_hint(dir: &str) -> u32 {
    dir.split('/')
        .filter_map(|seg| seg.split(['x', '@']).next()?.parse::<u32>().ok())
        .max()
        .unwrap_or(16)
        .min(9_999)
}

fn icon_data_uri(icon: &str) -> Option<String> {
    if icon.is_empty() {
        return None;
    }
    let path = if icon.starts_with('/') {
        PathBuf::from(icon)
    } else {
        icon_index().get(icon)?.clone()
    };
    let bytes = std::fs::read(&path).ok()?;
    if bytes.len() > 512 * 1024 {
        return None;
    }
    let mime = if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
    {
        "image/svg+xml"
    } else {
        "image/png"
    };
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// Nom et icône d'une appli à partir de sa clé (binaire), de son
/// `application.name` et de son `application.icon_name`. Mis en cache.
pub fn identify(key: &str, app_name: &str, icon_name: &str) -> AppIdentity {
    static CACHE: OnceLock<Mutex<HashMap<String, AppIdentity>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let ck = format!("{key}\u{1}{app_name}\u{1}{icon_name}");
    if let Some(hit) = cache.lock().get(&ck) {
        return hit.clone();
    }
    let k = key.to_lowercase();
    let n = app_name.to_lowercase();
    let entry = desktop_entries()
        .iter()
        .find(|e| e.ids.contains(&k))
        .or_else(|| {
            desktop_entries()
                .iter()
                .find(|e| !n.is_empty() && (e.name.to_lowercase() == n || e.ids.contains(&n)))
        });
    let id = AppIdentity {
        name: entry.map(|e| e.name.clone()),
        icon: [
            icon_name,
            entry.map(|e| e.icon.as_str()).unwrap_or(""),
            key,
            &k,
        ]
        .iter()
        .find_map(|i| icon_data_uri(i)),
    };
    cache.lock().insert(ck, id.clone());
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_hint_reads_dir_names() {
        assert_eq!(size_hint("/usr/share/icons/hicolor/48x48/apps"), 48);
        assert_eq!(size_hint("/x/WhiteSur/apps/scalable"), 16);
        assert_eq!(size_hint("/x/hicolor/256x256@2/apps"), 256);
    }
}
