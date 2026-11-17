@echo off
rem Builds VizX and gathers it into one portable folder (dist\VizX in the
rem repository unless a folder is given): runs package-prototype.ps1 next
rem to this file. The execution policy is bypassed for this one run only;
rem nothing in the system settings changes.
rem
rem Usage: double-click, or  "Package VizX.cmd" [destination folder]
if "%~1"=="" (
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0package-prototype.ps1"
) else (
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0package-prototype.ps1" -Destination "%~1"
)
if errorlevel 1 echo Packaging failed; see the messages above.
echo.
pause
