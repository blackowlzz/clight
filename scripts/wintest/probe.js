// Runs inside clight (selftest build) and logs why transitions may not move.
(async () => {
  const inv = window.__TAURI__.core.invoke
  const log = (m) => inv('selftest_log', { msg: typeof m === 'string' ? m : JSON.stringify(m) })
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
  try {
    await sleep(15000)
    const root = document.documentElement
    log({ ua: navigator.userAgent, vis: document.visibilityState, focus: document.hasFocus(), reduced: matchMedia('(prefers-reduced-motion: reduce)').matches, fx: { ...root.dataset }, dpr: devicePixelRatio })

    const frames = () => new Promise((r) => { let n = 0; const t0 = document.timeline.currentTime; const f = () => { n++; requestAnimationFrame(f) }; requestAnimationFrame(f); setTimeout(() => r({ raf: n, timeline: Math.round(document.timeline.currentTime - t0) }), 1000) })
    log({ frames: await frames() })

    const caret = [...document.querySelectorAll('.e-caret')][0]
    const curline = [...document.querySelectorAll('.e-curline')][0]
    const show = (el) => el && { style: el.style.transform, computed: getComputedStyle(el).transform, anims: el.getAnimations().map((a) => [a.constructor.name, a.playState, a.currentTime, a.transitionProperty]) }
    log({ caret: show(caret), curline: show(curline) })

    const all = document.getAnimations().map((a) => ({ kind: a.constructor.name, state: a.playState, t: a.currentTime, start: a.startTime, prop: a.transitionProperty ?? a.animationName, target: a.effect?.target ? a.effect.target.tagName + '.' + a.effect.target.className + '#' + a.effect.target.id : null, iter: a.effect?.getComputedTiming().iterations }))
    log({ animations: all.length, list: all.slice(0, 40) })

    async function testTransition(label) {
      const d = document.createElement('div')
      d.style.cssText = 'position:fixed;left:0;top:0;width:10px;height:10px;background:red;transition:transform 100ms linear;z-index:9999'
      document.body.append(d)
      await sleep(50)
      d.style.transform = 'translateX(100px)'
      await sleep(400)
      const tr = getComputedStyle(d).transform
      const a = d.animate([{ opacity: 0 }, { opacity: 1 }], { duration: 200 })
      await sleep(400)
      const waapi = [a.playState, a.currentTime]
      d.remove()
      log({ test: label, transition: tr, waapi, frames: await frames() })
    }
    await testTransition('as is')

    // Experiments: take things away one at a time and test again.
    const sheet = [...document.styleSheets].find((s) => s.href?.includes('style.css'))
    const rules = () => [...sheet.cssRules]
    const drop = (pred) => {
      let n = 0
      for (let i = sheet.cssRules.length - 1; i >= 0; i--) if (pred(sheet.cssRules[i])) { sheet.deleteRule(i); n++ }
      return n
    }
    log({ dropped: 'mask', n: drop((r) => /mask/.test(r.cssText)) })
    await testTransition('no mask')
    log({ dropped: 'tabs transition', n: drop((r) => /--fade/.test(r.cssText)) })
    await testTransition('no --fade')
    log({ dropped: '@property', n: drop((r) => r.constructor.name === 'CSSPropertyRule') })
    await testTransition('no @property')
    log({ dropped: 'backdrop/filter', n: drop((r) => /backdrop-filter|filter:/.test(r.cssText)) })
    await testTransition('no filter')
    log({ dropped: 'keyframes+animation', n: drop((r) => r.constructor.name === 'CSSKeyframesRule' || /animation/.test(r.cssText)) })
    await testTransition('no keyframes')
    for (const el of document.querySelectorAll('[style*="mask"]')) el.style.mask = el.style.webkitMask = ''
    await testTransition('no inline mask')
    for (const a of document.getAnimations()) a.cancel()
    await testTransition('animations cancelled')
    log({ caret: show(caret), curline: show(curline) })
  } catch (e) {
    log('error ' + e + ' ' + e.stack)
  }
  log('done')
})()
