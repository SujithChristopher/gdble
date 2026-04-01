@echo off
REM Build GdBLE for Windows x86_64 and deploy to both submodule and main project.

echo Building GdBLE for Windows x86_64...
cargo build --release --target x86_64-pc-windows-msvc
if errorlevel 1 (echo Build failed & exit /b 1)

set SRC=target\x86_64-pc-windows-msvc\release\gdble.dll
set LOCAL_BIN=addons\gdble\bin\windows-x86_64
set PROJECT_BIN=..\addons\gdble\bin\windows-x86_64

echo Copying to submodule addons...
if not exist "%LOCAL_BIN%" mkdir "%LOCAL_BIN%"
copy /Y "%SRC%" "%LOCAL_BIN%\gdble.dll"

echo Copying to main project addons...
if not exist "%PROJECT_BIN%" mkdir "%PROJECT_BIN%"
copy /Y "%SRC%" "%PROJECT_BIN%\gdble.dll"

echo.
echo Build complete!
echo   Submodule : %LOCAL_BIN%\gdble.dll
echo   Project   : %PROJECT_BIN%\gdble.dll
