# Helpers partagés par les hooks MixFlow.
# Chargé via dot-sourcing : . "$PSScriptRoot\_common.ps1"

$ErrorActionPreference = 'Stop'

# Les shells lancés par les hooks n'héritent pas toujours de cargo/node.
function Initialize-MixFlowPath {
    $extra = @(
        (Join-Path $env:USERPROFILE '.cargo\bin'),
        'C:\Program Files\nodejs'
    )
    foreach ($dir in $extra) {
        if ((Test-Path $dir) -and ($env:Path -notlike "*$dir*")) {
            $env:Path = "$dir;" + $env:Path
        }
    }
}

# Lit le payload JSON du hook sur stdin. Renvoie $null si illisible :
# un hook ne doit JAMAIS casser la session sur une entrée inattendue.
function Read-HookInput {
    try {
        $raw = [Console]::In.ReadToEnd()
        if ([string]::IsNullOrWhiteSpace($raw)) { return $null }
        return ($raw | ConvertFrom-Json)
    } catch {
        return $null
    }
}

# Racine projet : fournie par Claude Code, sinon déduite du script.
function Get-ProjectDir {
    if ($env:CLAUDE_PROJECT_DIR) { return $env:CLAUDE_PROJECT_DIR }
    return (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
}

# Refus bloquant : exit 2 + message sur stderr = Claude voit la raison
# et peut corriger son tir. exit 2 est le SEUL code qui bloque réellement —
# exit 1 est traité comme une erreur non bloquante et la commande passerait.
#
# L'encodage de sortie est forcé en UTF-8 : sans ça, PowerShell 5.1 écrit
# stderr dans la codepage ANSI de la console et Claude Code reçoit les
# accents en mojibake (« BLOQU? »). Le BOM du fichier ne règle que la
# lecture du script, pas l'écriture de sa sortie.
function Deny-Tool([string]$Reason) {
    [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
    [Console]::Error.WriteLine($Reason)
    exit 2
}
