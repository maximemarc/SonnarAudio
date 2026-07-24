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

## Sécurité des dépendances

Le job `audit` de la CI échoue sur tout avis de sévérité **haute** ou plus,
des deux côtés :

```powershell
npm audit --audit-level=high
cd src-tauri; cargo audit   # cargo install cargo-audit --locked
```

`cargo audit` ne fait que comparer `Cargo.lock` à la base RustSec : il ne
compile rien, d'où un job sur `ubuntu-latest` alors que le crate lui-même
ne se construit que sous Windows.

Corriger de préférence avec `npm audit fix` **sans** `--force`, pour rester
dans les bornes semver : une montée de version majeure imposée par
`--force` casse plus souvent qu'elle ne répare.

### Alertes Dependabot attendues (ne pas chercher à les corriger)

Dependabot analyse `Cargo.lock`, qui contient les dépendances de **toutes**
les plateformes. Or MixFlow ne se construit que sous Windows : toute la
pile GTK/GDK/glib de Tauri (le backend `webkit2gtk`, utilisé sur Linux
uniquement — Windows passe par WebView2) figure dans le lockfile sans
jamais être compilée dans le binaire livré.

C'est notamment le cas de **RUSTSEC-2024-0429** (`glib` < 0.20, unsoundness
dans `VariantStrIter`) : Dependabot signale lui-même ne pas pouvoir le
corriger, Tauri 2 épinglant la génération gtk-rs 0.18. Vérification :

```powershell
cd src-tauri
cargo tree --target x86_64-pc-windows-msvc --invert glib   # "nothing to print"
cargo tree --target x86_64-unknown-linux-gnu --invert glib # glib <- gtk <- tauri
```

Ces alertes peuvent être classées « not affected » sur GitHub. `cargo audit`
les remonte en simple avertissement (`unmaintained` / `unsound`) et sort
en 0, donc la CI reste verte — c'est voulu : la durcir mettrait le build au
rouge en permanence pour du code qui n'est même pas compilé.

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
