@echo off
rem  Hazync — double-click this.
rem
rem  WHY A .BAT AND NOT JUST "run python hazync_gui.py". Someone who does not use a terminal has no
rem  reason to know what a terminal is, and "open cmd, cd to the folder, type python ..." is four
rem  chances to give up before anything happens. This is one double-click.
rem
rem  It also solves a real Windows annoyance: `python` on a fresh machine often opens the Microsoft
rem  Store instead of running anything, because Windows ships an App Execution Alias stub. So the
rem  `py` launcher is tried FIRST — it is what the python.org installer registers and it does not
rem  have that behaviour.
rem
rem  pythonw.exe, not python.exe, so no black console window sits behind the app.

setlocal
cd /d "%~dp0"

rem  1. pyw — the WINDOWLESS launcher the python.org installer registers beside py.exe. Preferred,
rem     and immune to the Store stub.
rem
rem  ⛔⛔ THIS SAID `py -3 -w` UNTIL 2026-10-05, AND IT STARTED NOTHING AT ALL.
rem  There is no -w option on the py launcher and none on python either. `py` passes it straight
rem  through to python.exe, which answers `Unknown option: -w`, prints its usage and exits. On a
rem  double-click that is a console that appears and vanishes -- reported from a real machine as
rem  "a window popped up and then closed".
rem
rem  ⚠ It was the FIRST branch tried and `where py` succeeds on every standard python.org install,
rem  so the fallbacks below were never reached: it failed for everybody, not in some corner case.
rem  The windowless interpreter is a separate executable -- pyw.exe -- not a flag.
where pyw >nul 2>nul
if %errorlevel%==0 (
    start "" pyw -3 hazync_gui.py %*
    exit /b 0
)

rem  2. pythonw on PATH
where pythonw >nul 2>nul
if %errorlevel%==0 (
    start "" pythonw hazync_gui.py %*
    exit /b 0
)

rem  3. py with a console. A console sitting behind the window is ugly; not starting is worse.
where py >nul 2>nul
if %errorlevel%==0 (
    start "" py -3 hazync_gui.py %*
    exit /b 0
)

rem  4. python on PATH — a console will flash, which is better than not starting
where python >nul 2>nul
if %errorlevel%==0 (
    rem  ⚠ The Store stub answers `where python` too. If this is the stub it prints a notice and
    rem  exits, so say what to do rather than leaving a blank screen.
    python -c "import sys" >nul 2>nul
    if %errorlevel%==0 (
        start "" python hazync_gui.py %*
        exit /b 0
    )
)

echo.
echo   Hazync needs Python, and this machine does not have a working one.
echo.
echo   Install it from:   https://www.python.org/downloads/windows/
echo.
echo   On the first installer screen, tick "Add python.exe to PATH".
echo   Everything else can stay as it is. Then double-click Hazync.bat again.
echo.
echo   (If you typed `python` and the Microsoft Store opened, that is Windows'
echo    placeholder rather than Python. The installer above replaces it.)
echo.
pause
exit /b 1
