"""Runs the installed clight on a Windows runner, types into the editor and records what the
caret, the scroll and the animations do. Everything lands in C:\\out."""
import glob, json, os, subprocess, time, urllib.request
from playwright.sync_api import sync_playwright
import pyautogui

OUT = r"C:\out"
os.makedirs(OUT, exist_ok=True)
proj = r"C:\t\proj"
os.makedirs(proj, exist_ok=True)
with open(os.path.join(proj, "main.c"), "w", newline="\r\n") as f:
    f.write("#include <stdio.h>\n\n")
    for i in range(300):
        f.write(f"int f{i}(int x) {{ return x + {i}; }}\n")

exe = (glob.glob(os.path.expandvars(r"%LOCALAPPDATA%\**\clight.exe"), recursive=True) + glob.glob(r"C:\Program Files\**\clight.exe", recursive=True))[0]
print("exe", exe)
env = dict(os.environ, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9222")
proc = subprocess.Popen([exe, os.path.join(proj, "main.c")], env=env)

for _ in range(120):
    try:
        urllib.request.urlopen("http://127.0.0.1:9222/json/version", timeout=1)
        break
    except Exception:
        time.sleep(0.5)

PROBE = r"""
(() => {
  const ed = [...document.querySelectorAll('.e-ed')].find((e) => e.querySelector('.e-scroll:not([hidden])'))
  if (!ed) return { noEditor: true }
  const scroll = ed.querySelector('.e-scroll'), caret = ed.querySelector('.e-caret'), ime = ed.querySelector('.e-ime')
  const scrolled = [...document.querySelectorAll('*')].filter((e) => e.scrollTop || e.scrollLeft).map((e) => `${e.tagName}.${e.className}#${e.id} ${e.scrollTop},${e.scrollLeft}`)
  const cs = getComputedStyle(caret)
  return {
    t: performance.now(), scrollTop: scroll.scrollTop, styleT: caret.style.transform, compT: cs.transform,
    caretRect: caret.getBoundingClientRect().toJSON(), scrollRect: scroll.getBoundingClientRect().toJSON(),
    imeT: ime.style.transform, focus: document.activeElement === ime, anims: document.getAnimations().length,
    timeline: document.timeline.currentTime, vis: document.visibilityState, scrolled,
    transition: cs.transitionProperty + ' ' + cs.transitionDuration,
  }
})()
"""

SAMPLER = r"""
(() => {
  window.__log = []
  window.__raf = 0
  const tick = () => { window.__raf++; requestAnimationFrame(tick) }
  requestAnimationFrame(tick)
  setInterval(() => { window.__log.push(eval(window.__probeSrc)) }, 20)
})()
"""


def save(name, data):
    with open(os.path.join(OUT, name), "w") as f:
        json.dump(data, f, indent=1)


with sync_playwright() as p:
    browser = p.chromium.connect_over_cdp("http://127.0.0.1:9222")
    page = None
    for _ in range(60):
        pages = [pg for c in browser.contexts for pg in c.pages]
        if pages:
            page = pages[0]
            break
        time.sleep(0.5)
    page.wait_for_selector(".e-scroll:not([hidden])", timeout=60000)
    time.sleep(3)
    info = page.evaluate("""() => ({ ua: navigator.userAgent, dpr: devicePixelRatio, reduced: matchMedia('(prefers-reduced-motion: reduce)').matches,
      fx: { ...document.documentElement.dataset }, vis: document.visibilityState, w: innerWidth, h: innerHeight })""")
    print("info", json.dumps(info))
    save("info.json", info)
    page.screenshot(path=os.path.join(OUT, "01-start.png"))
    pyautogui.screenshot(os.path.join(OUT, "01-start-screen.png"))

    # Animation frames and the timeline over one second.
    rate = page.evaluate("""() => new Promise((r) => { let n = 0; const t0 = document.timeline.currentTime; const f = () => { n++; requestAnimationFrame(f) }; requestAnimationFrame(f); setTimeout(() => r({ frames: n, timeline: document.timeline.currentTime - t0 }), 1000) })""")
    print("rate", rate)

    page.evaluate("(src) => { window.__probeSrc = src }", PROBE)
    page.evaluate(SAMPLER)

    # Scroll to the middle and click on a row there.
    page.evaluate("""() => { const s = [...document.querySelectorAll('.e-scroll')].find((e) => !e.hidden); s.scrollTop = 120 * parseInt(getComputedStyle(document.documentElement).getPropertyValue('--lh')) }""")
    time.sleep(1)
    box = page.evaluate("""() => { const s = [...document.querySelectorAll('.e-scroll')].find((e) => !e.hidden); const r = s.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 } }""")
    page.mouse.click(box["x"], box["y"])
    time.sleep(1)
    print("after click", json.dumps(page.evaluate(PROBE)))
    page.screenshot(path=os.path.join(OUT, "02-clicked.png"))
    pyautogui.screenshot(os.path.join(OUT, "02-clicked-screen.png"))

    page.evaluate("() => { window.__log = [] }")
    for i, ch in enumerate("int abc = 1;"):
        page.keyboard.type(ch)
        time.sleep(0.03)
        page.screenshot(path=os.path.join(OUT, f"03-cdp-{i:02}.png"))
    time.sleep(1)
    save("log-cdp.json", page.evaluate("() => window.__log"))
    print("after cdp typing", json.dumps(page.evaluate(PROBE)))
    pyautogui.screenshot(os.path.join(OUT, "03-cdp-screen.png"))

    # Real keys from the system, as a person types.
    page.evaluate("() => { window.__log = [] }")
    shots = []
    for i, ch in enumerate(" xyz"):
        pyautogui.write(ch)
        time.sleep(0.05)
        pyautogui.screenshot(os.path.join(OUT, f"04-os-{i:02}-screen.png"))
    time.sleep(1)
    save("log-os.json", page.evaluate("() => window.__log"))
    print("after os typing", json.dumps(page.evaluate(PROBE)))
    page.screenshot(path=os.path.join(OUT, "04-os.png"))
    print("raf total", page.evaluate("() => window.__raf"))

proc.kill()
