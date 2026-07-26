---
name: windows-audio
description: Travailler sur l'intégration Windows Audio de MixFlow (src-tauri/src/winapps.rs) — COM/WASAPI, câbles VB-Audio, renommage d'endpoint, routage d'une application vers un périphérique (IAudioPolicyConfigFactory), détection des apps, icônes d'apps. À utiliser dès qu'une app n'est pas détectée, un routage échoue (E_INVALIDARG), un câble est mal apparié ou l'UI se fige.
---

# Intégration Windows Audio

Tout vit dans `src-tauri/src/winapps.rs`. C'est la partie la plus fragile du
produit : interfaces non documentées, endpoints qui disparaissent, COM
capricieux.

## Deux règles non négociables

### 1. Tout appel COM passe par `in_com_thread`

Les commandes Tauri synchrones tournent sur le thread principal STA, où
l'énumération de sessions **échoue silencieusement** (pas d'erreur, juste une
liste vide). `in_com_thread` spawn+join un thread par appel.

### 2. Le travail COM se fait HORS du verrou `state.config`

`in_com_thread` bloque le temps du round-trip. Le tenir sous le verrou `config`
fige n'importe quel fader ailleurs dans l'UI. Motif de référence :
`add_line` / `remove_line`, avec le triptyque
`plan_cable_bindings` → `resolve_cable_bindings` → `apply_cable_bindings`,
réconciliés **par id de ligne, jamais par index** (la config peut changer
pendant l'opération).

Corollaire : toute commande qui touche Windows Audio ou un dialogue natif doit
être `async` (`list_apps`, `assign_app_to_line`, `unassign_app_from_line`,
`import_config`, `export_config`).

## Câbles VB-Audio

Chaque ligne `kind == "app"` s'approprie un câble au démarrage
(`auto_bind_lines`) et **renomme le côté rendu au nom du canal**
(« Game (VB-Audio…) ») : le canal devient sélectionnable comme haut-parleur
dans Windows.

- L'appariement capture ↔ rendu se fait par le **suffixe d'adaptateur**
  `(VB-Audio …)` (`render_id_for_capture`), **jamais** par le nom complet —
  que MixFlow renomme lui-même.
- `LineConfig.cable_render_id` cache l'id MMDevice du rendu, mais il est
  **revalidé** par `winapps::render_device_active` avant usage : les ids ne
  survivent pas à une réinstallation de VB-Cable, et
  `SetPersistedDefaultAudioEndpoint` accepte sans broncher un endpoint
  fantôme — les apps se retrouvaient routées dans le vide, en silence.
- Un câble utilisé comme **sortie** (montage streamer / OBS) n'est plus
  considéré libre (`cable_used_as_output`, apparié par suffixe d'adaptateur) :
  `add_line` se l'appropriait, renommait l'endpoint que le bus référence par
  nom et tuait le flux stream, avec réinjection du mix diffusé dans une ligne.
- Le renommage peut être refusé sans droits admin. Ces erreurs remontent en
  `notice` — **non fatales**, ne pas les transformer en `Err`.

## Routage d'une app vers un périphérique

Interface non documentée `IAudioPolicyConfigFactory` (IID Win11
`ab3d4648-…`, approche EarTrumpet). Windows persiste l'assignation.

**Essayer TOUS les PID de l'exe, pas seulement ceux des sessions audio**
(`candidate_pids`). Les navigateurs Chromium (Brave, Chrome, Edge) jouent le
son depuis un processus utilitaire *sandboxé* dont Windows ne sait pas
rattacher l'identité à l'application : `SetPersistedDefaultAudioEndpoint` y
répond `E_INVALIDARG` (0x80070057). Seul le processus principal accepte la
route. Le code s'arrêtait aux PID de session dès que la liste était non vide,
donc le routage de Brave échouait systématiquement. On concatène désormais
sessions + processus vivants (dédoublonnés) et on n'échoue que si **aucun**
n'accepte.

Limite connue : l'IID pré-21H2 (vieux Windows 10) n'est pas encore géré.

## Détection des apps

Sessions audio WASAPI sur tous les endpoints de rendu actifs **+** liste
blanche d'apps connues en cours d'exécution (Discord, Spotify, navigateurs) —
utile quand l'app est silencieuse. Trace : `[mixflow] scan apps: N` sur stderr.

Les icônes réelles passent par `app_icon_data_uri` : `SHGetFileInfoW` +
`GetIconInfo` / `GetDIBits` → BMP en mémoire → data-URI base64, **caché par
exe** (`OnceLock<Mutex<HashMap<...>>>`). L'extraction GDI est trop lente pour
tourner à chaque poll de `list_apps` (10 s) — ne pas contourner le cache.

## Santé des périphériques

Le thread `mixflow-health` (3 s) compare la liste cpal actuelle aux
périphériques réellement configurés (lignes + bus) et émet
`device_warnings: string[]`. C'est le **seul** filet côté UI : `stream_err`
dans `engine.rs` ne fait qu'un `eprintln!`.

## Symptôme → cause

| Symptôme | Cause probable |
| --- | --- |
| Liste de sessions vide sans erreur | appel COM hors de `in_com_thread` |
| `E_INVALIDARG` (0x80070057) au routage | PID sandboxé — élargir `candidate_pids` |
| App routée mais aucun son | `cable_render_id` fantôme non revalidé |
| Flux OBS coupé après un `add_line` | câble de sortie considéré libre |
| Faders figés quelques secondes | COM sous le verrou `config`, ou commande synchrone |
| Mauvais câble apparié | appariement par nom complet au lieu du suffixe d'adaptateur |

## Vérification

```powershell
cd src-tauri; cargo clippy --all-targets -- -D warnings
```

Le crate lie WASAPI / Win32 COM : il ne compile **que sous Windows**. Le
comportement réel se vérifie à la main (router Brave, débrancher un casque en
cours de session, réinstaller VB-Cable).
