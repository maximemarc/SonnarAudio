---
description: Prépare un commit dans le style du dépôt (Conventional Commits, corps en français qui explique la CAUSE)
disable-model-invocation: true
allowed-tools:
  - Bash(git status *)
  - Bash(git diff *)
  - Bash(git log *)
  - Bash(git show *)
  - Bash(git branch *)
  - Bash(git add *)
  - Bash(git commit *)
  - Read
  - Grep
  - Glob
---

Prépare un commit pour les modifications en cours.

**Regarde d'abord** : `git status`, `git diff`, `git diff --staged`, et
`git log -5 --format='%B'` pour te recaler sur le ton du dépôt.

## Format imposé

Le hook `commit-msg` (commitlint) et la CI rejettent tout ce qui s'en écarte.

```
<type>(<scope>): <description au présent, minuscule, sans point final>
```

Types : `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`,
`ci`, `chore`, `revert`. Scopes déjà utilisés : `ui`, `winapps`, `engine`,
`eq`, `release`, `readme`.

## Le corps du message (c'est là que ce dépôt se distingue)

En **français**, et il explique **la cause**, pas la liste des fichiers
touchés. Le lecteur doit comprendre pourquoi le bug existait. Structure qui
revient dans l'historique :

1. Le symptôme observé, concret (message d'erreur exact, code HRESULT,
   ce que voyait l'utilisateur).
2. **La cause réelle** — avec l'extrait de code fautif si ça aide, et les
   chiffres constatés (« 16 processus brave.exe vivants, aucun tenté »).
3. Ce que fait le correctif.
4. Ce qui reste volontairement en l'état, et pourquoi.

Ne rédige pas un corps creux qui paraphrase le titre. S'il n'y a rien à
expliquer (bump de version, format), un titre seul suffit.

## Avant de committer

- Ne mets en scène **que** ce qui appartient à ce changement. Si le diff
  mélange deux sujets, propose de le découper.
- Ne stage jamais `src-tauri/.cargo/config.toml` (gitignoré, local à la
  machine) ni un secret.
- Les hooks tournent : `lint-staged` puis `commitlint`. S'ils échouent,
  corrige la cause — n'utilise pas `--no-verify`.

Termine le message par :

```
Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
```

Si la branche courante est `main`, signale-le et propose une branche de
travail avant de committer.

$ARGUMENTS
