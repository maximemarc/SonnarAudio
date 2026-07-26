# PreToolUse (Bash|PowerShell) — bloque les commandes qui déclenchent un
# piège documenté dans CLAUDE.md. Chaque refus explique la raison ET
# l'alternative, pour que l'agent puisse corriger seul.

. "$PSScriptRoot\_common.ps1"

$payload = Read-HookInput
if ($null -eq $payload) { exit 0 }

$cmd = $payload.tool_input.command
if ([string]::IsNullOrWhiteSpace($cmd)) { exit 0 }

# --- Piège encodage : PS 5.1 Get-Content -Raw + réécriture = mojibake ----
# Les sources du projet sont en UTF-8 sans BOM ; ce combo les corrompt.
if ($cmd -match '(?i)Get-Content' -and $cmd -match '(?i)-Raw' -and
    $cmd -match '(?i)(Set-Content|Out-File|>\s*[""'']?\S+\.(ts|tsx|rs|md|json|css))') {
    Deny-Tool @"
BLOQUÉ — piège d'encodage (CLAUDE.md).
PowerShell 5.1 : 'Get-Content -Raw' suivi de 'Set-Content'/'Out-File' corrompt
les fichiers UTF-8 sans BOM (mojibake sur les accents), et tout ce dépôt est
en UTF-8 sans BOM avec une UI en français.
=> Utiliser les outils Edit / Write pour modifier un fichier.
"@
}

# --- Contournement des hooks Git ----------------------------------------
if ($cmd -match '(?i)git\s+commit\b.*(--no-verify|\s-n\b)') {
    Deny-Tool @"
BLOQUÉ — 'git commit --no-verify' saute lint-staged et commitlint.
Le job commitlint de la CI rejettera le message de toute façon.
=> Corriger le lint / le message (Conventional Commits, voir CONTRIBUTING.md).
Si l'utilisateur demande explicitement ce contournement, qu'il le lance lui-même.
"@
}

# --- Fichier MAX_PATH local, gitignoré, non reconstructible en CI --------
if ($cmd -match '(?i)(Remove-Item|rm\b|del\b|git\s+clean).*\.cargo[\\/]config\.toml') {
    Deny-Tool @"
BLOQUÉ — src-tauri/.cargo/config.toml redirige le target cargo vers un chemin
court. Le supprimer casse la compilation ici (LNK1104, MAX_PATH) et il est
gitignoré : personne ne pourra le restaurer depuis le dépôt.
"@
}

# --- npm audit fix --force : casse plus qu'il ne répare ------------------
if ($cmd -match '(?i)npm\s+audit\s+fix\b.*--force') {
    Deny-Tool @"
BLOQUÉ — 'npm audit fix --force' sort des bornes semver et monte des majeures.
CONTRIBUTING.md impose 'npm audit fix' sans --force.
"@
}

# --- Force push ---------------------------------------------------------
if ($cmd -match '(?i)git\s+push\b.*(--force\b|-f\b)' -and $cmd -notmatch '--force-with-lease') {
    Deny-Tool @"
BLOQUÉ — force push. Utiliser au minimum '--force-with-lease', et seulement
sur une branche de travail, jamais sur main.
"@
}

# --- Serveur de dev en avant-plan : bloque la session pour rien ---------
if ($cmd -match '(?i)npm\s+run\s+tauri\s+dev' -or $cmd -match '(?i)^\s*npm\s+run\s+dev\s*$') {
    Deny-Tool @"
BLOQUÉ — 'tauri dev' ne rend jamais la main et fige la session.
=> Le lancer avec run_in_background, ou demander à l'utilisateur de l'ouvrir
   dans son propre terminal. Penser d'abord aux zombies sur le port 5173 :
   Get-NetTCPConnection -LocalPort 5173
"@
}

exit 0
