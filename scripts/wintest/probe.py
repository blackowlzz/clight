"""Runs the installed clight on a Windows runner and uses it with the system mouse and keyboard,
taking screenshots of the screen while typing. Everything lands in C:\\out."""
import os, subprocess, threading, time
import pyautogui

pyautogui.FAILSAFE = False
OUT = r"C:\out"
os.makedirs(OUT, exist_ok=True)
proj = r"C:\t\proj"
os.makedirs(proj, exist_ok=True)
with open(os.path.join(proj, "main.c"), "w", newline="\r\n") as f:
    f.write("#include <stdio.h>\n\n")
    for i in range(300):
        f.write(f"int f{i}(int x) {{ return x + {i}; }}\n")

exe = os.path.expandvars(r"%LOCALAPPDATA%\clight\clight.exe")
proc = subprocess.Popen([exe, os.path.join(proj, "main.c")])
time.sleep(20)
shot = lambda name: pyautogui.screenshot(os.path.join(OUT, name + ".png"))
shot("01-start")
w, h = pyautogui.size()
print("screen", w, h)

# The AI popup in the corner, if it is there.
pyautogui.click(892, 697)
time.sleep(1)


def burst(prefix, seconds):
    """Screenshots as fast as possible for a while, to catch a jump."""
    stop = time.time() + seconds
    n = 0
    while time.time() < stop:
        shot(f"{prefix}-{n:03}")
        n += 1


def typing(prefix, text):
    t = threading.Thread(target=burst, args=(prefix + "-burst", 3.0))
    t.start()
    for ch in text:
        pyautogui.write(ch)
        time.sleep(0.12)
    t.join()
    time.sleep(0.5)
    shot(prefix + "-end")


# Near the top first.
pyautogui.click(640, 300)
time.sleep(1)
shot("02-click-top")
typing("03-top", "abc")

# Then scrolled far down.
pyautogui.moveTo(640, 400)
for _ in range(15):
    pyautogui.scroll(-500)
    time.sleep(0.1)
time.sleep(1)
shot("04-scrolled")
pyautogui.click(640, 400)
time.sleep(1)
shot("05-click-down")
typing("06-down", "xyz q")
pyautogui.press("enter")
time.sleep(0.5)
shot("07-enter")
pyautogui.press("enter")
time.sleep(0.5)
shot("08-enter2")
proc.kill()
