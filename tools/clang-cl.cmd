@echo off
setlocal

REM Windows shim for the Unix `tools/clang-cl` wrapper.
REM Prefer `clang-cl` if present, otherwise fall back to MSVC `cl`.

where clang-cl >nul 2>nul
if %ERRORLEVEL%==0 (
  clang-cl %*
  exit /b %ERRORLEVEL%
)

cl %*
exit /b %ERRORLEVEL%
