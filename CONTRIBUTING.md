# Contribuer à MixFlow

## Mise en place

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;C:\Program Files\nodejs;" + $env:Path
npm install   # installe aussi les hooks Git via le script "prepare" (husky)
```

## Style de code

|                   | Outil                  | Commande                                                  |
| ----------------- | ---------------------- | --------------------------------------------------------- |
| Frontend (lint)   | ESLint 9 (flat config) | `npm run lint` / `npm run lint:fix`                       |
| Frontend (format) | Prettier               | `npm run format` / `npm run format:check`                 |
| Frontend (types)  | tsc                    | `npm run typecheck`                                       |
| Backend (format)  | rustfmt                | `cd src-tauri; cargo fmt`                                 |
| Backend (lint)    | clippy                 | `cd src-tauri; cargo clippy --all-targets -- -D warnings` |
| Backend (tests)   | cargo test             | `cd src-tauri; cargo test`                                |

Ces mêmes commandes tournent en CI (`.github/workflows/ci.yml`) sur chaque
pull request — une PR qui ne les passe pas en local ne passera pas en CI.

## Hooks Git (husky)

- **pre-commit** : `lint-staged` — ESLint + Prettier sur les fichiers TS/TSX
  modifiés, Prettier sur JSON/CSS/MD/HTML. Rapide, ne touche que le staged.
- **commit-msg** : `commitlint` — vérifie que le message suit
  [Conventional Commits](https://www.conventionalcommits.org/).

Pour committer sans passer par les hooks (déconseillé, cas exceptionnel) :
`git commit --no-verify`.

## Format des commits (Conventional Commits)

```
<type>(<scope optionnel>): <description au présent, minuscule, pas de point final>
```

Types acceptés : `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`,
`build`, `ci`, `chore`, `revert`.

Exemples :

```
feat(eq): ajoute la saisie numérique freq/gain par point
fix(engine): corrige le clic audible à l'ajout d'une bande EQ
docs(readme): met à jour le guide d'installation VB-Cable
ci: passe le job backend sur windows-latest
```

Un commit qui ne respecte pas ce format est rejeté par le hook `commit-msg`
et par le job `commitlint` en CI sur les pull requests.

## Pièges spécifiques à ce projet

Voir [CLAUDE.md](CLAUDE.md) — MAX_PATH, thread COM dédié, encodage
PowerShell, etc. À lire avant de toucher à `winapps.rs` ou `engine.rs`.

## Avant d'ouvrir une pull request

```powershell
npm run lint && npm run format:check && npm run typecheck && npm run build
cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
