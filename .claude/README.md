# Environnement Claude Code — MixFlow

Ce dossier configure Claude Code pour ce dépôt. Il est **versionné** (sauf
`settings.local.json`, gitignoré) : tout contributeur qui clone le projet
hérite des mêmes garde-fous.

```
.claude/
├── settings.json          permissions, variables d'env, branchement des hooks
├── agents/                sous-agents de relecture spécialisés
├── commands/              commandes /slash
├── hooks/                 scripts PowerShell déclenchés par le harnais
└── skills/                connaissances chargées à la demande
```

## Skills

Chargées automatiquement quand le sujet correspond à leur `description`.

| Skill | Couvre |
| --- | --- |
| `moteur-audio` | `src-tauri/src/audio/` — resampler, servo, EQ, ducking, contrat temps réel |
| `windows-audio` | `winapps.rs` — COM/WASAPI, câbles VB-Audio, routage par app, icônes |
| `commande-tauri` | ajouter une commande : topologie vs live, verrous, async, persistance |
| `ui-console` | `src/` — console, faders à VU, courbe EQ, drag & drop, métering |
| `publier-version` | versions, tag, `release.yml`, signature minisign, mise à jour auto |

## Commandes

| Commande | Effet | Invocable par Claude |
| --- | --- | --- |
| `/check` | toute la batterie pré-PR (front + back), puis correction de ce qui bloque | oui |
| `/diag-audio [symptôme]` | part du symptôme entendu et remonte au code | oui |
| `/audit-deps` | `npm audit` + `cargo audit`, avec le tri des faux positifs Linux-only | oui |
| `/dev` | démarre l'app en dev après avoir écarté PATH, port 5173 et MAX_PATH | non |
| `/commit` | commit au format du dépôt — Conventional Commits, corps qui explique la cause | non |
| `/release [version]` | aligne les 3 versions, vérifie, tague, rappelle l'étape manuelle | non |

Les trois dernières portent `disable-model-invocation: true` : démarrer un
serveur, écrire un commit ou pousser un tag sont des effets de bord dont
l'utilisateur choisit le moment. Claude ne peut pas les déclencher seul.

Le champ `allowed-tools` de chaque commande **pré-autorise** des outils le
temps du tour (il ne les restreint pas — c'est une permission, pas un
bac à sable). Il est donc réduit aux commandes réellement nécessaires :
`/commit` obtient `git add`/`git commit`, pas `Bash` en entier.

## Agents

Invoqués explicitement (`revue-temps-reel`, `revue-windows-audio`). Ils
relisent du code déjà écrit à la recherche de bugs propres à ce projet — pas
du style. Chacun précharge la skill correspondante (`skills:` en frontmatter)
pour démarrer avec les invariants du domaine déjà en contexte.

## Hooks

| Déclencheur | Script | Rôle |
| --- | --- | --- |
| `SessionStart` (`startup\|resume`) | `session-start.ps1` | signale ce qui est cassé localement : `.cargo/config.toml` manquant (MAX_PATH), cargo/node absents du PATH, `node_modules` absent, port 5173 occupé. Émet un `hookSpecificOutput.additionalContext`, et **rien du tout** si tout va bien. |
| `PreToolUse` (Bash, PowerShell) | `guard-bash.ps1` | bloque les commandes qui déclenchent un piège documenté : `Get-Content -Raw` + réécriture (mojibake UTF-8), `git commit --no-verify`, suppression de `.cargo/config.toml`, `npm audit fix --force`, force push, `tauri dev` en avant-plan. |
| `PostToolUse` (Edit, Write) | `format-file.ps1` | formate le fichier touché — `rustfmt` pour `.rs`, `prettier` pour `.ts/.tsx/.json/.css/.md/.html`. Ne bloque jamais. |

Les scripts partagent `hooks/_common.ps1` (lecture du payload JSON, PATH
cargo/node, refus bloquant). Un refus sort en **exit 2** avec la raison sur
stderr, pour que l'agent puisse corriger seul — c'est le seul code de sortie
qui bloque réellement (`exit 1` laisserait passer la commande).

Les hooks sont déclarés en **forme exec** (`command` + `args`) avec
`${CLAUDE_PROJECT_DIR}` : pas de quoting shell à gérer, donc pas de casse si
le dépôt migre vers un chemin contenant des espaces.

Les `.ps1` portent un **BOM UTF-8** : PowerShell 5.1 lit un `.ps1` sans BOM
en ANSI et les accents des messages sortent en mojibake. `.editorconfig`
l'impose (`[*.ps1] charset = utf-8-bom`).

### Désactiver temporairement un hook

Commenter son entrée dans `settings.json`, ou la surcharger dans
`.claude/settings.local.json` (gitignoré, propre à ta machine).

## Permissions

`settings.json` pré-autorise les commandes de vérification (lint, typecheck,
build, clippy, test, `git status`/`diff`/`log`) pour éviter les demandes
répétitives, et **interdit** la lecture des clés et fichiers `.env` ainsi que
les force push.

Pour ajouter une autorisation qui ne concerne que toi, la mettre dans
`.claude/settings.local.json` plutôt que dans le fichier versionné.
