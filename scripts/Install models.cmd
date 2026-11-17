@echo off
rem Opens the VizX model installer window (install-models.ps1 next to this
rem file). The execution policy is bypassed for this one run only; nothing
rem in the system settings changes.
start "" powershell.exe -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "%~dp0install-models.ps1" %*
