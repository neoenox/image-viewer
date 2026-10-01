# Opens Windows Settings > Default apps so the user can assign file types
# to this viewer with a few clicks (Windows itself writes valid hashes).
# Requires associate.ps1 to have been run first. No admin needed.

$ErrorActionPreference = "Stop"

Start-Process "ms-settings:defaultapps"
"Opened Settings > Default apps."
"Pick this viewer for each image type. It appears as the registered app name."
