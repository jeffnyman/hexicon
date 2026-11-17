@echo off
rem Build the reference TADS 3 toolchain and test programs into
rem build\reference at the repository root. Requires Visual Studio 2022
rem (or Build Tools) with the C++ workload, and CMake.
setlocal

set "ROOT=%~dp0..\.."
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%VSWHERE%" (
    echo vswhere.exe not found; is Visual Studio installed?
    exit /b 1
)
for /f "usebackq delims=" %%i in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VSDIR=%%i"
if not defined VSDIR (
    echo No Visual Studio installation with the C++ tools was found.
    exit /b 1
)
call "%VSDIR%\VC\Auxiliary\Build\vcvars64.bat" >nul || exit /b 1

cmake -G Ninja -S "%~dp0." -B "%ROOT%\build\reference" -DCMAKE_BUILD_TYPE=Release %* || exit /b 1
cmake --build "%ROOT%\build\reference" || exit /b 1
