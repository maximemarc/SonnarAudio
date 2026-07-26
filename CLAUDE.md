# MixFlow — CLAUDE.md

Application desktop Windows de routage/mixage audio virtuel, clone de
SteelSeries Sonar. Tauri 2 (Rust) + React/Vite. UI en français.

## Outillage Claude Code (`.claude/`)

Le détail est dans [.claude/README.md](.claude/README.md). En résumé :

- **Skills** chargées automatiquement selon le sujet : `moteur-audio`,
  `windows-audio`, `commande-tauri`, `ui-console`, `publier-version`.
- **Commandes** : `/check` (batterie pré-PR), `/dev`, `/commit`,
  `/diag-audio <symptôme>`, `/audit-deps`, `/release <version>`.
- **Agents** de relecture : `revue-temps-reel`, `revue-windows-audio`.
- **Hooks** : contrôle d'environnement au démarrage, blocage des commandes
  qui déclenchent un piège ci-dessous (mojibake PowerShell, `--no-verify`,
  suppression de `.cargo/config.toml`…), formatage automatique après édition.

Ce fichier reste la source de vérité sur l'architecture ; les skills en
détaillent des morceaux et ne s'y substituent pas.

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
de commit + un job `audit` (`npm audit --audit-level=high` et `cargo audit`,
sur `ubuntu-latest` : `cargo audit` ne lit que `Cargo.lock`, il ne compile
rien). `release.yml` construit l'installeur NSIS sur un tag `vX.Y.Z`.
Hooks locaux (husky) : `pre-commit` = lint-staged, `commit-msg` = commitlint
(Conventional Commits, voir CONTRIBUTING.md).

## Pièges connus (à lire avant de toucher quoi que ce soit)

- **MAX_PATH** : ce dossier de session fait ~200 caractères. Le target cargo
  est redirigé vers un chemin court via `src-tauri/.cargo/config.toml`. Ne
  pas supprimer ce fichier tant que le projet vit ici (sinon LNK1104).
  Il est **gitignoré** : un `target-dir` absolu n'a de sens que sur la
  machine qui l'a écrit et casserait la compilation des autres et de la CI.
  À recréer à la main si besoin :

  ```toml
  [build]
  target-dir = "C:/un/chemin/court"
  ```

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
  Le servo s'asservit sur le **minimum des rings VIVANTS** : un ring resté
  vide plus de `DEAD_AFTER_EMPTY_BLOCKS` blocs (producteur mort — capture
  qui a échoué ou flux tué) est exclu, sinon son zéro permanent épinglait
  le trim et faisait saturer les rings sains du même bus.
- **Phase du resampler** : `Resampler` garde **deux** frames d'historique
  (`prev`/`prev2`). Côté rendu, la position résiduelle repart négative dès
  que `step < 1` (périphérique de sortie au-dessus de 48 kHz) ; la clamper
  à 0 jetait de la phase à chaque bloc — crachotement périodique audible.
  Test de non-régression : `exact_preserves_phase_across_blocks_when_upsampling`.
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
  `try_read` dans le rendu. Une source **muette** ne ducke rien (le mute
  s'applique au rendu, l'enveloppe étant calculée avant — sans ce test,
  Ctrl+Alt+M coupait le micro mais continuait de baisser le jeu). Et comme
  `env` n'est décrémentée que par le callback de capture, un `capture_tick`
  sert de battement de cœur : le thread `mixflow-levels` relâche
  l'enveloppe quand le compteur stagne (flux mort), sinon les cibles
  restaient atténuées à vie.
- **Master global** : `master_gain` (config) multiplié sur chaque bus de
  sortie dans le rendu.
- **Persistance débouncée** : les commandes _live_ posent juste un flag
  `AppState.dirty` (`persist()`) ; un thread dédié (`mixflow-persist`)
  flush sur disque toutes les ~800 ms, plus un flush best-effort au clic
  "Quitter" du tray. Les commandes _topologie_ (`rebuild()`) écrivent tout
  de suite — pas de debounce, elles sont peu fréquentes.
  `save_config` écrit en **tmp + rename atomique**, et toute écriture se
  fait **sous le verrou `config`** : trois threads peuvent la déclencher
  (main, `mixflow-persist`, `mixflow-profiles`) et deux écritures
  entrelacées produisaient un JSON tronqué, que `load_or_default`
  remplaçait en silence par la config d'usine au démarrage suivant.
- **`rebuild()` tient le verrou `config` de bout en bout** (clone → save →
  swap des `Controls` → envoi au moteur). Sans ça, deux rebuilds
  concurrents pouvaient laisser le moteur câblé sur un plan de contrôle
  que plus aucune commande live n'écrivait : faders/mutes/EQ sans effet.
  Aucun appelant ne doit donc déjà détenir ce verrou (parking_lot n'est
  pas réentrant).
- **`sanitize_config`** : tout ce qui vient du disque ou d'un import y
  passe. Un JSON valide n'est pas sain — `"gain": 1e300` devient `+inf`
  en f32, contamine le lissage de gain du rendu puis l'état des biquads
  (ligne muette ou bruyante jusqu'au rebuild). La fonction borne
  gains/EQ/ducking, dédoublonne ids et routes, purge les références
  orphelines et applique la migration EQ legacy.
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
- **Essayer TOUS les PID de l'exe, pas seulement ceux des sessions audio**
  (`candidate_pids`) : les navigateurs Chromium (Brave, Chrome, Edge) jouent
  le son depuis un processus utilitaire _sandboxé_, dont Windows ne sait pas
  rattacher l'identité à l'application — `SetPersistedDefaultAudioEndpoint`
  y répond `E_INVALIDARG` (0x80070057). Seul le processus principal accepte
  la route. Le code s'arrêtait aux PID de session dès que la liste était non
  vide : le routage de Brave échouait donc à tous les coups. On concatène
  désormais sessions + processus vivants (dédoublonnés) et on n'échoue que
  si aucun n'accepte.
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
  `set_route`, `set_output_device`, `rename_output`, `set_output_gain`,
  `set_output_muted` ont été supprimés (relents de l'ancienne UI
  matrice/bus visibles, plus aucun appelant côté UI). `set_route_gain`
  existe à nouveau (réintroduit) : gain par sortie d'UNE ligne (pas par
  bus global), exposé dans `ChannelStrip` seulement à partir de 2 sorties
  simultanées (fan-out) — voir `LineCtl.routes: HashMap<output_id,
RouteCtl>` dans `controls.rs`, déjà lu dans `RenderState::render`
  avant même que la commande existe.
- **Mode Streamer** : pas de second bus de mix dans le moteur — juste la
  commande `enable_streamer_mode(device)` qui ajoute ce périphérique comme
  route supplémentaire sur CHAQUE ligne (find-or-create bus), puis
  s'appuie sur le gain par sortie ci-dessus pour équilibrer mix perso vs
  mix diffusé. Zéro changement dans `engine.rs`.
- **Profils** (`AppConfig.profiles: Vec<Profile>`) : snapshot par ligne
  (gain/mute/EQ/sorties par NOM de périphérique, pas par bus id — voir
  `LineSnapshot`) + ducking + master. `apply_profile_to_config` (main.rs)
  résout les sorties via `find_or_create_bus`, ignore les lignes/règles
  dont l'id n'existe plus. Thread `mixflow-profiles` (2 s) compare
  `winapps::foreground_exe()` au `trigger_exe` de chaque profil et
  applique automatiquement (émet `config_updated` — voir plus bas).
  Le **profil actif vit dans `AppState.active_profile`**, pas dans une
  variable locale au thread : `apply_profile` (manuel) l'y écrit aussi,
  sinon le thread ré-appliquait 2 s plus tard le profil que l'utilisateur
  venait de poser à la main. `save_profile` ne photographie **pas** les
  bus sans périphérique (`device: ""`), qu'`apply_profile_to_config`
  filtre ensuite — la ligne se serait retrouvée sans aucune sortie.
- **Raccourci global Ctrl+Alt+M** (`tauri-plugin-global-shortcut`) :
  bascule le mute de TOUTES les lignes `kind == "mic"` d'un coup (état
  "any unmuted" → tout couper, sinon tout réactiver). Live (atomics),
  pas de rebuild. Comme ce déclencheur ne vient pas de l'UI, il émet
  `config_updated` (payload `AppConfig`) — App.tsx écoute cet événement
  pour rester synchro, même mécanisme utilisé par l'auto-switch de
  profil ci-dessus.
- **Santé des périphériques** : thread `mixflow-health` (3 s) compare la
  liste cpal actuelle aux périphériques réellement configurés (lignes +
  bus) et émet `device_warnings: string[]` (mergé dans le même
  `warn-banner` que `engine_status` côté frontend) quand un périphérique
  configuré disparaît en cours de session — `stream_err` (engine.rs) ne
  fait toujours qu'un `eprintln!`, ce thread est le seul filet côté UI.
- **Ducking réglable** : `LineConfig.duck_reactivity` ("douce" | "normale"
  | "rapide") pilote `LineCtl.duck_decay` (coefficient de décroissance du
  suiveur d'enveloppe, appliqué dans `CaptureState::process`). Propriété
  de la ligne SOURCE, pas de la règle — exposé dans `DuckingPanel` à côté
  de chaque règle mais modifie la ligne source globalement.
- **Export/import config** (`tauri-plugin-dialog`, dialogues natifs
  appelés côté Rust via `blocking_save_file`/`blocking_pick_file` — pas
  de package npm, tout passe par les commandes `export_config`/
  `import_config`, même convention que le reste de l'app).
- **Icônes d'apps réelles** (`winapps::app_icon_data_uri`) :
  `SHGetFileInfoW` + `GetIconInfo`/`GetDIBits` → BMP en mémoire → data-URI
  base64, caché par exe (`OnceLock<Mutex<HashMap<...>>>` — l'extraction
  GDI est trop lente pour tourner à chaque poll de `list_apps` à 10 s).
- **Mise à jour auto** : `tauri-plugin-updater` enregistré, `check_for_update`
  vérifie / télécharge / installe pour de vrai. Le plugin **refuse de
  démarrer** sans `plugins.updater` valide dans `tauri.conf.json` (pubkey +
  endpoints) — il panique tout `main()`, testé : ne pas retirer ce bloc.
  Signature minisign vérifiée contre la pubkey avant écriture, donc un
  endpoint compromis ne suffit pas à installer un binaire arbitraire. La
  clé PRIVÉE vit uniquement dans les secrets GitHub (`release.yml`), jamais
  dans le dépôt. Procédure de publication : CONTRIBUTING.md.
- Les commandes qui font du **COM ou de l'I/O bloquant sont `async`**
  (`list_apps`, `assign_app_to_line`, `unassign_app_from_line`,
  `import_config`, `export_config`) : une commande Tauri **synchrone**
  s'exécute sur le thread principal tao, où un round-trip COM lent gelait
  faders, tray et fenêtre. Règle à suivre pour toute nouvelle commande qui
  touche Windows Audio ou un dialogue natif.
- Un **câble virtuel servant de SORTIE** (montage streamer/OBS) n'est plus
  considéré libre par `plan_cable_bindings`/`assign_app_to_line`
  (`cable_used_as_output`, apparié par suffixe d'adaptateur) : `add_line`
  se l'appropriait, renommait l'endpoint que le bus référence par nom et
  tuait le flux stream, avec réinjection du mix diffusé dans une ligne.
- **`cable_render_id` est revalidé** (`winapps::render_device_active`)
  avant usage du cache : les ids MMDevice ne survivent pas à une
  réinstallation de VB-Cable, et `SetPersistedDefaultAudioEndpoint`
  accepte sans broncher un endpoint fantôme — les apps étaient routées
  dans le vide, en silence.
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

- Support de l'IID AudioPolicyConfig pré-21H2 pour vieux Windows 10.
- Personnalisation du raccourci global (Ctrl+Alt+M est câblé en dur dans
  `main.rs`, pas encore configurable depuis l'UI).
- Un profil peut désigner une app absente/désinstallée comme déclencheur
  sans erreur (comparaison silencieuse, jamais de match) — acceptable
  pour l'instant mais pas signalé à l'utilisateur.
