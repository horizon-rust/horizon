@echo off
setlocal

set "REPO_URL=https://github.com/melthedevv/horizon.git"
set "BRANCH=main"

rem Work in the folder this .bat file is located in
cd /d "%~dp0"

where git >nul 2>&1
if errorlevel 1 (
    echo Git is not installed or not in PATH. Get it from https://git-scm.com/downloads
    pause
    exit /b 1
)

rem Fix "dubious ownership" error (happens on drives like E: that don't record ownership)
set "SAFE_DIR=%CD:\=/%"
git config --global --get-all safe.directory | findstr /x /c:"%SAFE_DIR%" >nul 2>&1
if errorlevel 1 (
    git config --global --add safe.directory "%SAFE_DIR%"
)

if not exist ".git" (
    echo Initializing new git repository...
    git init
)

rem Set git identity if none is configured yet
git config user.name >nul 2>&1
if errorlevel 1 git config --global user.name "melthedevv"
git config user.email >nul 2>&1
if errorlevel 1 git config --global user.email "fettahbacak2@gmail.com"

git branch -M %BRANCH%

git remote get-url origin >nul 2>&1
if errorlevel 1 (
    git remote add origin %REPO_URL%
) else (
    git remote set-url origin %REPO_URL%
)

echo Adding files...
git add -A

git diff --cached --quiet
if errorlevel 1 (
    git commit -m "Upload %date% %time%"
    if errorlevel 1 (
        echo.
        echo Commit failed. If git asks who you are, run these once:
        echo   git config --global user.name "Your Name"
        echo   git config --global user.email "you@example.com"
        pause
        exit /b 1
    )
) else (
    echo No new changes to commit.
)

echo Pushing to %REPO_URL% ...
git push -u origin %BRANCH%
if errorlevel 1 (
    echo.
    echo Push was rejected. The repo probably already has files that this folder doesn't.
    set /p FORCE="Force push and overwrite the remote? (y/N): "
    if /i "%FORCE%"=="y" (
        git push -u --force origin %BRANCH%
        if errorlevel 1 (
            echo Force push failed.
            pause
            exit /b 1
        )
    ) else (
        echo Cancelled.
        pause
        exit /b 1
    )
)

echo.
echo Done! Uploaded to %REPO_URL%
pause
endlocal