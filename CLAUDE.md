# MixFlow — CLAUDE.md

Application desktop Windows de routage/mixage audio virtuel, clone de
SteelSeries Sonar. Tauri 2 (Rust) + React/Vite. UI en français.

## Commandes

```powershell
# PATH (les shells n'ont pas toujours cargo/node) :
$env:Path = "$env:USERPROFILE\.cargo\bin;C:\Program Files\nodejs;" + $env:Path

npm run tauri dev          # dev (vite HMR + cargo watch), port 5173 strict
npx tsc                    # type-check frontend
npx vite build             # build frontend
cd src-tauri; cargo check  # check backend
cd src-tauri; cargo test   # tests DSP (resampler, soft-clip, EQ, sanitize_bands)
npm run tauri build        # installeur NSIS

# Lint / format (voir CONTRIBUTING.md pour le détail complet) :
npm run lint               # eslint .
npm run format:check       # prettier --check .
cd src-tauri; cargo fmt --check
cd src-tauri; cargo clippy --all-targets -- -D warnings
```

CI/CD (`.github/workflows/`) : `ci.yml` fait tourner ces mêmes commandes sur
chaque PR (frontend sur `ubuntu-latest`, backend sur `windows-latest` —
obligatoire, le crate lie WASAPI/Win32 COM) + `commitlint` sur les messages
de commit. `release.yml` construit l'installeur NSIS sur un tag `vX.Y.Z`.
Hooks locaux (husky) : `pre-commit` = lint-staged, `commit-msg` = commitlint
(Conventional Commits, voir CONTRIBUTING.md).

## Pièges connus (à lire avant de toucher quoi que ce soit)

- **MAX_PATH** : ce dossier de session fait ~200 caractères. Le target cargo
  est redirigé vers `C:\Users\maxim\mixflow-target` via
  `src-tauri/.cargo/config.toml`. Ne pas supprimer ce fichier tant que le
  projet vit ici (sinon LNK1104).
- **COM** : tout appel COM de `src-tauri/src/winapps.rs` DOIT passer par
  `in_com_thread` (les commandes Tauri synchrones tournent sur le thread
  principal STA — l'énumération de sessions y échoue silencieusement).
- **Encodage** : PowerShell 5.1 `Get-Content -Raw` + `Set-Content` corrompt
  les fichiers UTF-8 sans BOM (mojibake) — utiliser les outils Edit/Write.
- **Vite 8 (rolldown)** : pas de `minify: "esbuild"` dans vite.config.ts.
- **Port 5173** : tuer les zombies node avant de relancer
  (`Get-NetTCPConnection -LocalPort 5173`).
- **Drag & drop HTML** : ne fonctionne que parce que
  `"dragDropEnabled": false` est posé sur la fenêtre dans tauri.conf.json.
- Le renommage de périphérique Windows peut être refusé sans admin — les
  erreurs remontent en `notice` (non fatales).

## Architecture

```
Frontend React (src/)  ── invoke ──▶  main.rs (commandes, état, persistance)
       ▲                                   │ mpsc topologie      │ atomics live
       └── events "levels" 20 Hz,          ▼                     ▼
           "engine_status"          engine.rs (thread dédié, possède les
                                    cpal::Stream !Send)
```

- **Domaine canonique 48 kHz stéréo** ; resampler linéaire streaming
  (`dsp.rs`), ring buffers lock-free par route (`ringbuf`). Servo de dérive
  dans `RenderState::render` (±0.3 % de trim sur le resampler) pour garder
  le remplissage des rings calé sur `PREFILL_FRAMES` (~30 ms) sans craquer.
- **Deux familles de commandes** (main.rs) : _topologie_ (add/remove,
  devices, routes → rebuild complet du moteur) et _live_ (gains, mutes, EQ,
  ducking, master → atomics dans `controls.rs`, zéro glitch).
- **EQ paramétrique par ligne** : 1..10 biquads peaking (`LineConfig.
eq_bands`, freq 20-20k / gain ±12 dB, Q=1, écart mini `MIN_BAND_FREQ_RATIO`
  entre points imposé par `sanitize_bands`) appliqués dans le callback de
  capture ; bandes derrière un `RwLock` lu en `try_read` par bloc. Quand le
  nombre de points change (ajout/suppression), les filtres sont réconciliés
  par fréquence (pas juste recréés) pour ne pas perdre l'état des bandes
  inchangées et éviter un clic. Champ `eq` = legacy 5 bandes, migré au
  démarrage.
- **Ducking** : enveloppe par ligne (capture) + règles source→cible lues en
  `try_read` dans le rendu.
- **Master global** : `master_gain` (config) multiplié sur chaque bus de
  sortie dans le rendu.
- **Persistance débouncée** : les commandes _live_ posent juste un flag
  `AppState.dirty` (`persist()`) ; un thread dédié (`mixflow-persist`)
  flush sur disque toutes les ~800 ms, plus un flush best-effort au clic
  "Quitter" du tray. Les commandes _topologie_ (`rebuild()`) écrivent tout
  de suite — pas de debounce, elles sont peu fréquentes.
- **`schema_version`** (`AppConfig`) : bumpé après les migrations de
  démarrage. Les migrations actuelles se basent sur la forme des données
  (`eq_bands.is_empty()`, etc.) et n'en ont pas besoin, mais une future
  migration qui doit tourner une seule fois quel que soit l'état peut se
  gater sur `schema_version < N`.

## Modèle audio Windows (le cœur du produit)

- Les **canaux d'apps** (`LineConfig.kind == "app"`) s'approprient chacun un
  **câble VB-Audio** au démarrage (`auto_bind_lines`) et **renomment le côté
  rendu du câble au nom du canal** (« Game (VB-Audio…) ») → les canaux sont
  sélectionnables comme haut-parleurs dans Windows.
- L'appariement capture↔rendu d'un câble se fait par le **suffixe
  d'adaptateur** `(VB-Audio …)` (`render_id_for_capture`) — jamais par le
  nom complet, que MixFlow lui-même renomme. L'id MMDevice du rendu est
  caché dans `LineConfig.cable_render_id`.
- Le **routage par app** (drag-drop) utilise l'interface non documentée
  `IAudioPolicyConfigFactory` (IID Win11 `ab3d4648-…`, style EarTrumpet) :
  Windows persiste l'assignation app→périphérique.
- La **détection d'apps** = sessions audio WASAPI sur tous les endpoints de
  rendu actifs + liste blanche d'apps connues en cours d'exécution
  (Discord, Spotify, navigateurs… — utile quand l'app est silencieuse).
  Log stderr : `[mixflow] scan apps: N`.
- Le **canal Micro** (`kind == "mic"`) capture un périphérique physique.
  La capture **loopback** d'un périphérique de rendu est supportée par le
  moteur (fallback dans `build_capture`) mais n'est plus exposée dans l'UI.
- Les **bus de sortie** existent dans le moteur mais sont invisibles dans
  l'UI : `set_line_outputs(line_id, devices: Vec<String>)` fait
  find-or-create un bus par périphérique choisi (**fan-out possible** — une
  ligne peut jouer sur plusieurs sorties physiques à la fois, ex. casque +
  haut-parleurs) et `prune_unused_buses` nettoie les bus devenus orphelins.
  Chaque tranche a sa section « Sortie(s) » (chips + menu "+ ajouter").
  `set_line_output` (singulier), `add_output`, `remove_output`,
  `set_route`, `set_output_device`, `rename_output`, `set_route_gain`,
  `set_output_gain`, `set_output_muted` ont été supprimés (relents de
  l'ancienne UI matrice/bus visibles, plus aucun appelant côté UI).
- **`add_line`/`remove_line`** font le travail COM bloquant (`winapps.rs`,
  qui spawn+join un thread par appel) **hors du verrou** `state.config` —
  motif à respecter pour toute nouvelle commande qui touche Windows Audio,
  sinon un fader ailleurs dans l'UI peut geler le temps de la réponse COM.
  Voir `plan_cable_bindings` / `resolve_cable_bindings` /
  `apply_cable_bindings` (réconciliées par **id de ligne**, jamais par
  index, pour rester correctes si la config change pendant l'opération).

## UI (console façon Sonar)

Deux onglets (state `view` dans App.tsx) :

- **Console** : **MASTER** (fader global + zone « À router » des apps non
  routées) | **Canaux** (Game/Chat/Media : sortie, EQ mini, fader à VU
  intégré, zone Apps drop-target) + **ChatMix** | **Micro**. Ducking dessous.
- **Égaliseur** (`EqPage.tsx`) : canal sélectionnable, courbe SVG
  interactive (drag des points = freq+gain, double-clic = ajout, clic
  droit = suppression ; maths partagées dans `eqMath.ts`, mêmes formules
  RBJ que `dsp.rs`), + saisie numérique précise par point (`EqPointChip`,
  tampon local + commit au blur/Enter, même motif que le nom de canal).
  Pendant le drag, `dragBands` (état local) donne le retour visuel instantané
  pendant que l'appel réseau reste throttlé (60 ms) — sans ça un drag rapide
  peut se perdre ou saccader (le round-trip backend est plus lent que les
  événements pointeur). Presets d'usine (`BUILTIN_EQ_PRESETS`, presets
  distincts pour les micros : `BUILTIN_MIC_EQ_PRESETS`) + presets custom
  persistés (`AppConfig.eq_presets`, commandes `set_line_eq_bands` /
  `save_eq_preset` / `delete_eq_preset`). Les tranches montrent une
  mini-courbe cliquable qui ouvre l'onglet.
- VU dans la piste du fader (`VSlider` prop `meter`, id `"*"` = agrégat).
- Halo « du son passe ici » : `useLiveLevel` pilote `--live` (opacité).
- Tout le métering passe par rAF hors React (`levels.ts`).
- Fermer la fenêtre = réduction dans le tray (menu Ouvrir/Quitter).
- Config persistée : `%AppData%\com.mixflow.app\mixflow.config.json`.

## Qualité / CI

- **Frontend** : ESLint 9 flat config (`eslint.config.js`, TS + React Hooks
  - React Refresh) + Prettier (`.prettierrc.json`). `pickIcon` vit dans son
    propre fichier (`pickIcon.tsx`, pas `Icons.tsx`) pour éviter l'avertissement
    react-refresh/only-export-components — respecter ce découpage pour tout
    nouvel export non-composant.
- **Backend** : `rustfmt.toml` + `cargo clippy --all-targets -- -D warnings`
  (CI échoue sur le moindre warning clippy, pas seulement les erreurs).
- **Commits** : Conventional Commits, imposé par le hook `commit-msg`
  (commitlint) et par le job CI sur chaque PR — voir CONTRIBUTING.md.
- Les workflows GitHub Actions (`.github/workflows/`) sont écrits mais ce
  repo n'a pas (encore) de remote GitHub configuré.

## Reste à faire / idées

- Mode Streamer (double mix personnel/diffusé).
- Icônes réelles des apps (SHGetFileInfo) dans la zone « À router ».
- Support de l'IID AudioPolicyConfig pré-21H2 pour vieux Windows 10.
- Gain par sortie dans l'UI multi-sortie (le backend le préserve déjà par
  périphérique dans `set_line_outputs`, juste pas de slider dédié encore).
