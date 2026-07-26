---
name: commande-tauri
description: Ajouter ou modifier une commande Tauri dans MixFlow (src-tauri/src/main.rs + src/api.ts) — choix topologie vs live, verrous, async, persistance débouncée, événements config_updated, sanitize_config, profils. À utiliser dès qu'il faut exposer une nouvelle action du backend au frontend ou toucher à AppState.
---

# Ajouter une commande Tauri

Une commande MixFlow, c'est cinq endroits cohérents entre eux :

1. `src-tauri/src/main.rs` — la fonction `#[tauri::command]`
2. `main.rs` — l'entrée dans `generate_handler![...]`
3. `src/api.ts` — le wrapper `invoke` typé
4. `src/types.ts` — les types partagés, si la forme des données change
5. `sanitize_config` — si la commande écrit une valeur persistée

## Étape 1 : topologie ou live ?

C'est la seule vraie décision, et elle détermine tout le reste.

**Topologie** — ajout/suppression de ligne, périphériques, routes.
Reconstruit le moteur, écrit la config **tout de suite** (pas de debounce,
c'est rare). Passe par `rebuild()`.

**Live** — gains, mutes, EQ, ducking, master. Écrit dans les atomics de
`controls.rs`, zéro glitch audio, pose juste le flag `AppState.dirty` via
`persist()`. Le thread `mixflow-persist` flush sur disque toutes les ~800 ms,
plus un flush best-effort au clic « Quitter » du tray.

En cas de doute : est-ce que ça change le **graphe** (qui est connecté à quoi)
ou juste une **valeur** sur un graphe inchangé ? Graphe → topologie.

## Étape 2 : les verrous

`rebuild()` tient le verrou `config` **de bout en bout** (clone → save → swap
des `Controls` → envoi au moteur). Sans ça, deux rebuilds concurrents peuvent
laisser le moteur câblé sur un plan de contrôle que plus aucune commande live
n'écrit : faders, mutes et EQ deviennent sans effet.

Conséquence : **aucun appelant de `rebuild()` ne doit déjà détenir le verrou
`config`.** `parking_lot` n'est pas réentrant — ce serait un deadlock.

Trois threads peuvent déclencher une écriture (main, `mixflow-persist`,
`mixflow-profiles`). `save_config` écrit donc en **tmp + rename atomique**, et
toute écriture se fait **sous le verrou `config`** : deux écritures entrelacées
produisaient un JSON tronqué, que `load_or_default` remplaçait en silence par
la config d'usine au démarrage suivant.

## Étape 3 : sync ou async ?

Une commande Tauri **synchrone** s'exécute sur le thread principal tao.
Un round-trip COM ou un dialogue natif y gèle faders, tray et fenêtre.

> Toute commande qui touche Windows Audio ou un dialogue natif est `async`.

Déjà async : `list_apps`, `assign_app_to_line`, `unassign_app_from_line`,
`import_config`, `export_config`. Et le travail COM se fait hors du verrou
`config` — voir la skill `windows-audio`.

## Étape 4 : qui déclenche ?

Si le déclencheur **ne vient pas de l'UI**, le frontend ne peut pas deviner
que l'état a changé : émettre `config_updated` (payload `AppConfig`). `App.tsx`
écoute cet événement. C'est déjà le cas pour le raccourci global Ctrl+Alt+M et
pour l'auto-switch de profil (`mixflow-profiles`, 2 s).

## Étape 5 : la valeur est-elle persistée ?

Alors elle passe par `sanitize_config` : bornes, dédoublonnage d'ids et de
routes, purge des références orphelines. Un JSON valide n'est pas sain —
`"gain": 1e300` devient `+inf` en f32 et contamine le rendu.

Si une future migration doit tourner **une seule fois quel que soit l'état**,
la gater sur `schema_version < N` (`AppConfig`). Les migrations actuelles se
basent sur la forme des données (`eq_bands.is_empty()`) et n'en ont pas besoin.

## Sorties et bus

Les bus existent dans le moteur mais sont invisibles dans l'UI.
`set_line_outputs(line_id, devices: Vec<String>)` fait find-or-create un bus
par périphérique (**fan-out** : une ligne peut jouer sur casque + haut-parleurs
à la fois), `prune_unused_buses` nettoie les orphelins.

`set_route_gain` donne le gain par sortie d'**une** ligne (pas par bus global) —
`LineCtl.routes: HashMap<output_id, RouteCtl>` dans `controls.rs`. Exposé dans
`ChannelStrip` seulement à partir de 2 sorties simultanées.

Le **mode Streamer** n'ajoute aucun code moteur : `enable_streamer_mode(device)`
ajoute ce périphérique comme route supplémentaire sur chaque ligne, puis
s'appuie sur le gain par sortie pour équilibrer mix perso vs mix diffusé.

Supprimées, ne pas les ressusciter : `set_line_output` (singulier),
`add_output`, `remove_output`, `set_route`, `set_output_device`,
`rename_output`, `set_output_gain`, `set_output_muted`.

## Profils

`AppConfig.profiles` : snapshot par ligne (gain/mute/EQ/sorties **par nom de
périphérique**, pas par bus id — voir `LineSnapshot`) + ducking + master.

- `apply_profile_to_config` résout les sorties via `find_or_create_bus` et
  ignore les lignes/règles dont l'id n'existe plus.
- Le profil actif vit dans `AppState.active_profile`, **pas** dans une variable
  locale au thread : `apply_profile` (manuel) l'y écrit aussi, sinon le thread
  ré-appliquait 2 s plus tard le profil que l'utilisateur venait de poser.
- `save_profile` ne photographie **pas** les bus sans périphérique
  (`device: ""`), qu'`apply_profile_to_config` filtre ensuite : la ligne se
  retrouverait sans aucune sortie.

## Vérification

```powershell
cd src-tauri; cargo clippy --all-targets -- -D warnings; cargo test
```

```powershell
npm run typecheck
```

Le typecheck attrape la désynchronisation `api.ts` ↔ `types.ts`, mais **pas**
un décalage entre la signature Rust et le wrapper TS : relire les deux côte à
côte (noms de champs en camelCase côté invoke, sérialisation serde côté Rust).
