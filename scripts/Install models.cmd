@echo off
setlocal EnableExtensions
rem VizX model installer. Downloads the machine learning models VizX uses
rem (and ONNX Runtime, which runs two of them) from where their authors
rem publish them, checks every file against its SHA-256, and puts it where
rem VizX looks for it:
rem
rem   <VizX folder>\models\htdemucs_fp16weights.onnx    stem separation
rem   <VizX folder>\models\ggml-*.bin                     lyrics recognition
rem   <VizX folder>\models\voxceleb_resnet34_LM.onnx      telling singers apart
rem   <VizX folder>\onnxruntime*.dll                      runs HTDemucs and WeSpeaker
rem
rem Usage: double-click this file next to vizx.exe, or
rem   "Install models.cmd" [VizX folder]
rem
rem Uses only tools that come with Windows 10 (1803 and later) and 11:
rem curl.exe, tar.exe and certutil. Nothing is installed system-wide and
rem nothing needs administrator rights. An interrupted download continues
rem where it stopped the next time.
rem
rem This file must keep Windows (CRLF) line endings; see .gitattributes.

title VizX - Install Models
echo VizX - Install Models
echo =====================
echo Downloads the machine learning models VizX uses, from where their
echo authors publish them. Every file is checked (SHA-256) before it is used.
echo Everything runs on this computer.
echo.

for %%T in (curl.exe tar.exe certutil.exe) do (
    where %%T >nul 2>nul || (
        echo %%T was not found. It comes with Windows 10 version 1803 and later.
        goto :end
    )
)

rem --- The VizX folder ------------------------------------------------------

set "FOLDER=%~1"
if not defined FOLDER if exist "%~dp0vizx.exe" set "FOLDER=%~dp0"
if defined FOLDER goto :have_folder
echo There is no vizx.exe next to this file.
set /p "FOLDER=VizX folder (the one with vizx.exe): "
if not defined FOLDER goto :end
:have_folder
set "FOLDER=%FOLDER:"=%"
if "%FOLDER:~-1%"=="\" set "FOLDER=%FOLDER:~0,-1%"
if exist "%FOLDER%\" goto :folder_exists
echo The folder "%FOLDER%" does not exist.
goto :end
:folder_exists
if exist "%FOLDER%\vizx.exe" goto :folder_ok
echo There is no vizx.exe in "%FOLDER%".
echo VizX only finds the models next to its executable.
choice /c YN /m "Install here anyway"
if errorlevel 2 goto :end
:folder_ok
echo Installing into "%FOLDER%"
echo.

rem --- What to install ------------------------------------------------------

call :ask DEMUCS  "models\htdemucs_fp16weights.onnx" 165612636 "HTDemucs, stem separation (166 MB)"
call :ask ONNX    "onnxruntime.dll" 20602168 "ONNX Runtime 1.32.0, runs HTDemucs and WeSpeaker (93 MB download)"
call :ask TURBO   "models\ggml-large-v3-turbo-q5_0.bin" 574041195 "Whisper large-v3-turbo, lyrics recognition, recommended (574 MB)"
call :ask SPEAKER "models\voxceleb_resnet34_LM.onnx" 26530309 "WeSpeaker ResNet34, tells singers apart (27 MB)"
call :ask SMALL   "models\ggml-small.bin" 0 "Whisper small, faster but less accurate lyrics (466 MB, not needed with large-v3-turbo)"

set "INSTALLED=0"
set "FAILED=0"

if "%DEMUCS%"=="1" call :file "HTDemucs" ^
    "https://huggingface.co/StemSplitio/htdemucs-onnx/resolve/main/htdemucs_fp16weights.onnx" ^
    "models\htdemucs_fp16weights.onnx" ^
    d05c269d0178d2a72ad484b10b11dd370193fc923201c3b27a99f848745db70a

if "%ONNX%"=="1" call :onnx

if "%TURBO%"=="1" call :file "Whisper large-v3-turbo" ^
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin" ^
    "models\ggml-large-v3-turbo-q5_0.bin" ^
    394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2

if "%SPEAKER%"=="1" call :file "WeSpeaker ResNet34" ^
    "https://huggingface.co/Wespeaker/wespeaker-voxceleb-resnet34-LM/resolve/main/voxceleb_resnet34_LM.onnx" ^
    "models\voxceleb_resnet34_LM.onnx" ^
    7bb2f06e9df17cdf1ef14ee8a15ab08ed28e8d0ef5054ee135741560df2ec068

if "%SMALL%"=="1" call :whisper_small

echo.
echo =====================
if "%FAILED%"=="0" (
    echo Done: %INSTALLED% installed. Restart VizX if it is open.
) else (
    echo %INSTALLED% installed, %FAILED% failed; see the messages above.
    echo Run this again to retry; downloads continue where they stopped.
)
goto :end

rem --- Subroutines ----------------------------------------------------------

rem :ask VARIABLE target size description
rem Sets VARIABLE to 1 when the component should be installed. A file of
rem the expected size (any size when the size is 0) counts as installed.
:ask
set "%~1=0"
set "SIZE="
if exist "%FOLDER%\%~2" for %%A in ("%FOLDER%\%~2") do set "SIZE=%%~zA"
if not defined SIZE goto :ask_question
if "%~3"=="0" goto :ask_installed
if "%SIZE%"=="%~3" goto :ask_installed
:ask_question
choice /c YN /m "Install %~4"
if not errorlevel 2 set "%~1=1"
exit /b 0
:ask_installed
echo Already installed: %~4
exit /b 0

rem :file name url target sha256
:file
set "NAME=%~1"
set "TARGET=%FOLDER%\%~3"
set "EXPECTED=%~4"
echo.
echo --- %NAME% ---
for %%D in ("%TARGET%") do if not exist "%%~dpD" mkdir "%%~dpD"
call :download "%~2" "%TARGET%.part"
if errorlevel 1 goto :file_check_anyway
call :check "%TARGET%.part"
if errorlevel 1 goto :failed
goto :file_move
:file_check_anyway
rem A part that was already complete makes the server refuse to continue it.
if not exist "%TARGET%.part" goto :download_failed
call :hash "%TARGET%.part"
if /i not "%HASH%"=="%EXPECTED%" goto :download_failed
:file_move
move /y "%TARGET%.part" "%TARGET%" >nul
if errorlevel 1 goto :failed
echo Installed %TARGET%
set /a INSTALLED+=1
exit /b 0

:download_failed
echo The download of %NAME% stopped. Run this again to continue where it stopped.
:failed
set /a FAILED+=1
exit /b 1

rem :download url part - continues a part left from before.
:download
echo Downloading %~1
curl.exe -L --fail --retry 3 --retry-delay 5 -C - -o "%~2" "%~1"
exit /b %ERRORLEVEL%

rem :hash file - sets HASH to the file's SHA-256.
:hash
echo Checking the file...
set "HASH="
for /f "delims=" %%H in ('certutil -hashfile "%~1" SHA256 ^| findstr /v /c:":"') do if not defined HASH set "HASH=%%H"
if defined HASH set "HASH=%HASH: =%"
exit /b 0

rem :check file - compares with EXPECTED; deletes a file that does not match.
:check
call :hash "%~1"
if /i "%HASH%"=="%EXPECTED%" exit /b 0
del /f /q "%~1" >nul 2>nul
echo %NAME% is damaged or not the expected file.
echo   SHA-256 %HASH%
echo   expected %EXPECTED%
echo It was deleted; run this again to download it once more.
exit /b 1

rem Whisper small is checked against the hash its Hugging Face repository
rem publishes (the Git LFS pointer of the file).
:whisper_small
set "SMALL_SHA="
for /f "tokens=2 delims=:" %%S in ('curl.exe -sL --fail "https://huggingface.co/ggerganov/whisper.cpp/raw/main/ggml-small.bin" ^| findstr /c:"oid sha256:"') do set "SMALL_SHA=%%S"
if defined SMALL_SHA goto :whisper_small_go
echo.
echo --- Whisper small ---
echo Could not read the published checksum of Whisper small.
set /a FAILED+=1
exit /b 1
:whisper_small_go
call :file "Whisper small" ^
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin" ^
    "models\ggml-small.bin" ^
    %SMALL_SHA%
exit /b %ERRORLEVEL%

rem ONNX Runtime: only onnxruntime.dll and onnxruntime_providers_shared.dll
rem are taken out of the release archive (92 MB to download, 21 MB installed).
:onnx
set "NAME=ONNX Runtime"
set "ZIP=%FOLDER%\onnxruntime-download.zip"
set "UNPACK=%FOLDER%\onnxruntime-unpack"
echo.
echo --- %NAME% ---
call :download "https://github.com/microsoft/onnxruntime/releases/download/v1.32.0/onnxruntime-win-x64-1.32.0.zip" "%ZIP%.part"
if not errorlevel 1 goto :onnx_unpack
rem A complete archive (its table of contents is at the end) is used anyway.
if not exist "%ZIP%.part" goto :download_failed
tar.exe -tf "%ZIP%.part" >nul 2>nul
if errorlevel 1 goto :download_failed
:onnx_unpack
move /y "%ZIP%.part" "%ZIP%" >nul
if exist "%UNPACK%" rd /s /q "%UNPACK%"
mkdir "%UNPACK%"
echo Unpacking...
tar.exe -xf "%ZIP%" -C "%UNPACK%"
if errorlevel 1 goto :onnx_failed
call :onnx_place onnxruntime.dll 348c3d29d6995eee37e3451825eaddb7ea7400be3afda6ca2286b12bc775f042
if errorlevel 1 goto :onnx_failed
call :onnx_place onnxruntime_providers_shared.dll 6caf57ac07c9cf72fb4e73c5bb0fe156f02dd60bbbd7385530c9a5654f10f950
if errorlevel 1 goto :onnx_failed
rd /s /q "%UNPACK%"
del /f /q "%ZIP%"
echo Installed onnxruntime.dll and onnxruntime_providers_shared.dll
set /a INSTALLED+=1
exit /b 0
:onnx_failed
if exist "%UNPACK%" rd /s /q "%UNPACK%"
del /f /q "%ZIP%" >nul 2>nul
goto :failed

rem :onnx_place name sha256 - checks one DLL from the archive and puts it
rem next to vizx.exe.
:onnx_place
set "FOUND="
for /f "delims=" %%F in ('dir /s /b "%UNPACK%\%~1" 2^>nul') do if not defined FOUND set "FOUND=%%F"
if defined FOUND goto :onnx_place_found
echo %~1 is not in the ONNX Runtime archive.
exit /b 1
:onnx_place_found
set "EXPECTED=%~2"
call :check "%FOUND%"
if errorlevel 1 exit /b 1
copy /y "%FOUND%" "%FOLDER%\%~1" >nul
exit /b %ERRORLEVEL%

:end
echo.
pause
endlocal
