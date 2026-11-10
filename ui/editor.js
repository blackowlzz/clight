import { invoke, $, keyName, root } from './core.js'
import * as theme from './theme.js'

const scroll = $('scroll'), sizer = $('sizer'), gutter = $('gutter'), linesEl = $('lines'), gmarks = $('gmarks')
const dropCaret = $('drop-caret')
const caret = $('caret'), curline = $('curline'), execline = $('execline'), selEl = $('sel'), marksEl = $('marks')
const diagsEl = $('diags'), occEl = $('occ'), pairEl = $('pair'), ime = $('ime')

let LH = 20, CH = 8, GW = 56
const PAD = 8
const FOLD_W = 16

let docId = null
let queue = []
let busy = false
let wantPaint = false
let reveal = false
let clip = ''
let lastTop = -1
let onView = () => {}
let idleWaiters = []
let view = null
let sizerH = ''
let sizerW = ''
let lastCaret = null
const trail = $('caret-trail')
const caretGlow = $('caret-glow')
const ghostEl = $('ghost')
let ghostText = ''
let seenRev = -1, seenTotal = -1, seenDoc = null, seenRow = -1, seenCol = 0
let typedMarks = []
const TYPED_MS = 260

const live = new Map()
const spare = []

const rows = () => Math.max(1, Math.floor(scroll.clientHeight / LH))

export const metrics = () => ({ LH, CH })

export function configure({ onChange }) {
  onView = onChange
}

export function measure() {
  const m = $('measure')
  m.textContent = 'x'.repeat(100)
  CH = m.getBoundingClientRect().width / 100
  root.style.setProperty('--ch', CH + 'px')
  LH = parseInt(getComputedStyle(document.documentElement).getPropertyValue('--lh'), 10) || 20
  for (const r of live.values()) recycle(r)
  live.clear()
  repaint()
}

async function pump() {
  if (busy || docId === null) return
  busy = true
  try {
    while ((queue.length || wantPaint) && docId !== null) {
      const cmds = queue
      const id = docId
      const rev = reveal
      queue = []
      wantPaint = false
      reveal = false
      const top = Math.floor(scroll.scrollTop / LH)
      lastTop = top
      const v = await invoke('doc_apply', { id, cmds, top, rows: rows(), reveal: rev })
      if (id === docId) paint(v, rev)
    }
  } catch (e) {
    console.error(e)
  } finally {
    busy = false
    for (const w of idleWaiters.splice(0)) w()
  }
}

export function send(...cmds) {
  queue.push(...cmds)
  reveal = true
  pump()
}

export function quiet(...cmds) {
  queue.push(...cmds)
  pump()
}

export function repaint() {
  wantPaint = true
  pump()
}

export function idle() {
  return !busy && !queue.length ? Promise.resolve() : new Promise((r) => idleWaiters.push(r))
}

export function current() {
  return docId
}

export const lastView = () => view

const scrollMemo = new Map()

export function show(id) {
  if (docId !== null) scrollMemo.set(docId, [scroll.scrollTop, scroll.scrollLeft])
  docId = id
  queue = []
  scroll.hidden = id === null
  $('empty').hidden = id !== null
  if (id === null) return
  const [t, l] = scrollMemo.get(id) ?? [0, 0]
  for (const r of live.values()) recycle(r)
  live.clear()
  sizer.style.height = sizerH = '0'
  view = null
  lastCaret = null
  if (root.dataset.fxUi === 'on') linesEl.animate([{ opacity: 0.35 }, { opacity: 1 }], { duration: 120, easing: 'ease-out' })
  queue.push({ t: 'fresh' })
  wantPaint = true
  pump().then(() => {
    scroll.scrollTop = t
    scroll.scrollLeft = l
  })
  focus()
}

export function forget(id) {
  scrollMemo.delete(id)
}

export function focus() {
  ime.focus({ preventScroll: true })
}

export const hasFocus = () => document.activeElement === ime

function recycle(r) {
  r.l.hidden = true
  r.g.hidden = true
  spare.push(r)
}

function setHtml(el, key, html) {
  if (el[key] !== html) {
    el[key] = html
    el.innerHTML = html
  }
}

function paint(v, revealed) {
  view = v
  const digits = String(v.total).length
  const gw = Math.ceil(Math.max(digits, 3) * CH) + 22 + FOLD_W
  if (gw !== GW) {
    GW = gw
    document.documentElement.style.setProperty('--gw', gw + 'px')
  }
  const height = v.n * LH + 'px'
  if (height !== sizerH) sizer.style.height = sizerH = height
  const width = GW + PAD + (v.maxw + 4) * CH + 'px'
  if (width !== sizerW) sizer.style.width = sizerW = width

  if (revealed && v.top !== Math.floor(scroll.scrollTop / LH)) {
    scroll.scrollTop = v.top * LH
    lastTop = v.top
  }

  const end = v.first + v.lines.length
  for (const [n, r] of live) {
    if (n < v.first || n >= end) {
      recycle(r)
      live.delete(n)
    }
  }
  const [curR, curV] = v.cur
  for (let i = 0; i < v.lines.length; i++) {
    const n = v.first + i
    let r = live.get(n)
    if (!r) {
      r = spare.pop()
      if (!r) {
        r = { l: document.createElement('div'), g: document.createElement('div') }
        r.l.className = 'l'
        r.g.className = 'gl'
        linesEl.appendChild(r.l)
        gutter.appendChild(r.g)
      }
      const y = n * LH + 'px'
      r.l.style.top = y
      r.g.style.top = y
      r.l.style.left = PAD + 'px'
      r.l.hidden = false
      r.g.hidden = false
      live.set(n, r)
    }
    const num = v.nums[i] + 1
    if (r.num !== num) {
      r.num = num
      r.g.textContent = num
    }
    setHtml(r.l, '_h', v.lines[i])
    const on = n === curR
    if (r.on !== on) {
      r.on = on
      r.g.classList.toggle('cur', on)
    }
  }

  const x = PAD + curV * CH
  const y = curR * LH
  const tr = `translate(${x}px,${y}px)`
  smear(x, y)
  typingEffects(v, x, y, curR)
  if (ghostText) ghostEl.style.transform = tr
  caret.style.transform = tr
  ime.style.transform = tr
  curline.style.transform = `translateY(${y}px)`
  execline.hidden = v.exec === null
  if (v.exec !== null) execline.style.transform = `translateY(${v.exec * LH}px)`

  const rect = (list) => list.map(([l, a, b]) => `<div style="top:${l * LH}px;left:${PAD + a * CH}px;width:${(b - a) * CH}px"></div>`).join('')
  setHtml(selEl, '_h', rect(v.sel))
  setHtml(marksEl, '_h', rect(v.marks))
  setHtml(occEl, '_h', rect(v.occ))
  setHtml(pairEl, '_h', v.pair.map(([l, a]) => `<div style="top:${l * LH}px;left:${PAD + a * CH}px;width:${CH}px"></div>`).join(''))
  setHtml(diagsEl, '_h', v.diags.map(([l, a, b, s]) => `<div class="${s === 1 ? 'e' : 'w'}" style="top:${(l + 1) * LH - 2}px;left:${PAD + a * CH}px;width:${(b - a) * CH}px"></div>`).join(''))

  const stripeMax = Math.max(v.n, 1)
  setHtml($('stripes'), '_h', v.stripes.map(([r, s]) => `<i class="${s === 1 ? 'e' : 'w'}" style="top:${(r / stripeMax) * 100}%"></i>`).join(''))

  let g = ''
  for (const [row, kind] of v.git) g += `<i class="g${kind}" style="top:${row * LH}px"></i>`
  for (const row of v.bps) g += `<i class="bp" style="top:${row * LH}px"></i>`
  if (v.exec !== null) g += `<i class="ex" style="top:${v.exec * LH}px"></i>`
  for (const [row, s] of v.folds) g += `<i class="fd${s === 2 ? ' closed' : ''}" data-row="${row}" style="top:${row * LH}px">${s === 2 ? '›' : '⌄'}</i>`
  setHtml(gmarks, '_h', g)

  if (revealed) {
    const margin = CH * 3
    const left = scroll.scrollLeft
    const width = scroll.clientWidth - GW
    if (x - margin < left) scroll.scrollLeft = Math.max(0, x - margin)
    else if (x + margin > left + width) scroll.scrollLeft = x + margin - width
  }

  if (v.clip !== null && v.clip !== undefined) clip = v.clip
  onView(docId, v)
}

function typingEffects(v, x, y, row) {
  const same = seenDoc === docId
  const edited = same && v.rev !== seenRev
  const grew = same && v.total === seenTotal + 1
  const prevRow = seenRow, prevCol = seenCol
  seenDoc = docId
  seenRev = v.rev
  seenTotal = v.total
  seenRow = row
  seenCol = v.cur[1]
  if (root.dataset.fxType !== 'on') return
  const size = LH * 7
  caretGlow.style.width = caretGlow.style.height = size + 'px'
  caretGlow.style.transform = `translate(${x - size / 2 + 1}px,${y + LH / 2 - size / 2}px)`
  if (!edited) return
  caretGlow.getAnimations().forEach((a) => a.cancel())
  caretGlow.animate([{ opacity: 1 }, { opacity: 0.5 }], { duration: 420, easing: 'ease-out' })
  const typed = v.cur[1] - prevCol
  if (!grew && prevRow === row && typed > 0 && typed <= 80) typedMarks.push({ row, from: prevCol, to: v.cur[1], t: performance.now() })
  fadeTyped(row)
  if (grew) live.get(row)?.l.animate([{ opacity: 0.2, transform: 'translateY(-5px)' }, { opacity: 1, transform: 'none' }], { duration: 150, easing: 'ease-out' })
}

/** Text you just wrote fades in with the same soft blur as the text the AI writes. */
function fadeTyped(row) {
  const now = performance.now()
  typedMarks = typedMarks.filter((m) => m.row === row && now - m.t < TYPED_MS)
  const el = live.get(row)?.l
  if (!el || !typedMarks.length) return
  const nodes = []
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT)
  let offset = 0
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    nodes.push([n, offset])
    offset += n.data.length
  }
  const jobs = []
  for (const m of typedMarks) {
    nodes.forEach(([node, start], i) => {
      const a = Math.max(m.from, start) - start
      const b = Math.min(m.to, start + node.data.length) - start
      if (b > a) jobs.push({ i, node, a, b, age: now - m.t })
    })
  }
  jobs.sort((p, q) => q.i - p.i || q.a - p.a)
  for (const j of jobs) {
    const span = document.createElement('span')
    span.className = 'typed'
    span.style.animationDelay = `-${Math.round(j.age)}ms`
    const range = document.createRange()
    range.setStart(j.node, j.a)
    range.setEnd(j.node, j.b)
    range.surroundContents(span)
  }
}

/** `ahead` is how many characters the caret is about to move, until the next repaint catches up. */
export function setGhost(text, ahead = 0) {
  ghostText = text
  ghostEl.textContent = text
  ghostEl.hidden = !text
  if (text && lastCaret) ghostEl.style.transform = `translate(${lastCaret.x + ahead * CH}px,${lastCaret.y}px)`
}

export function flashSaved() {
  if (root.dataset.fxType !== 'on') return
  scroll.animate(
    [{ boxShadow: 'inset 0 0 0 1px color-mix(in srgb, var(--accent) 55%, transparent)' }, { boxShadow: 'inset 0 0 0 1px transparent' }],
    { duration: 520, easing: 'ease-out' },
  )
}

function smear(x, y) {
  const from = lastCaret
  lastCaret = { x, y }
  if (!from || root.dataset.fxTrail !== 'on') return
  const dx = from.x - x
  const dy = from.y - y
  const length = Math.hypot(dx, dy)
  if (!length || length > 700) return
  const angle = Math.atan2(dy, dx)
  const height = Math.abs(Math.cos(angle)) * LH + Math.abs(Math.sin(angle)) * 3
  trail.style.width = length + 'px'
  trail.style.height = height + 'px'
  trail.style.transform = `translate(${x}px,${y + (LH - height) / 2}px) rotate(${angle}rad)`
  trail.style.transformOrigin = `0 ${height / 2}px`
  trail.getAnimations().forEach((a) => a.cancel())
  trail.animate([{ opacity: 0.32 }, { opacity: 0 }], { duration: 130, easing: 'ease-out' })
}

let scrollQueued = false
scroll.addEventListener(
  'scroll',
  () => {
    hook.scrolled()
    if (scrollQueued) return
    scrollQueued = true
    requestAnimationFrame(() => {
      scrollQueued = false
      if (Math.floor(scroll.scrollTop / LH) !== lastTop) repaint()
    })
  },
  { passive: true },
)

new ResizeObserver(() => docId !== null && repaint()).observe(scroll)

function pointToCell(e) {
  const r = scroll.getBoundingClientRect()
  const y = e.clientY - r.top + scroll.scrollTop
  const x = e.clientX - r.left + scroll.scrollLeft - GW - PAD
  return { line: Math.max(0, Math.floor(y / LH)), vcol: Math.max(0, Math.round(x / CH)) }
}

export function caretBox() {
  if (!view) return { x: 0, y: 0, lh: LH }
  return { x: GW + PAD + view.cur[1] * CH - scroll.scrollLeft, y: view.cur[0] * LH - scroll.scrollTop, lh: LH }
}

const hook = {
  beforeKey: () => false,
  afterType: () => {},
  typed: () => {},
  ctrlClick: () => {},
  pointer: () => {},
  scrolled: () => {},
  context: () => {},
  gutterContext: () => {},
  hover: () => {},
  hoverEnd: () => {},
}

export function hooks(h) {
  Object.assign(hook, h)
}

let dragging = false
let moving = null
let lastPoint = null
let edgeTimer = 0

function dragTo(e) {
  lastPoint = e
  const c = pointToCell(e)
  queue.push({ t: 'click', line: c.line, vcol: c.vcol, ext: true })
  pump()
}

function edgeScroll() {
  edgeTimer = 0
  if ((!dragging && !moving?.active) || !lastPoint) return
  const r = scroll.getBoundingClientRect()
  const dy = lastPoint.clientY < r.top ? -LH * 2 : lastPoint.clientY > r.bottom ? LH * 2 : 0
  if (dy) {
    scroll.scrollTop += dy
    if (moving) showDrop(lastPoint)
    else dragTo(lastPoint)
    edgeTimer = setTimeout(edgeScroll, 40)
  }
}

function inSelection(e) {
  if (!view) return false
  const r = scroll.getBoundingClientRect()
  const c = pointToCell(e)
  const vcol = Math.floor((e.clientX - r.left + scroll.scrollLeft - GW - PAD) / CH)
  return view.sel.some(([l, a, b]) => l === c.line && vcol >= a && vcol < b)
}

function showDrop(e) {
  lastPoint = e
  const c = pointToCell(e)
  const row = view ? Math.min(c.line, view.n - 1) : c.line
  const length = live.get(row)?.l.textContent.length ?? c.vcol
  dropCaret.style.transform = `translate(${PAD + Math.min(c.vcol, length) * CH}px,${row * LH}px)`
  dropCaret.hidden = false
}

function endMove() {
  moving = null
  dropCaret.hidden = true
  scroll.classList.remove('moving')
}

const lastDown = { time: -Infinity, x: 0, y: 0, clicks: 0 }

// WebView2 always reports detail 0 on pointer events, so the count has to be ours.
function countClicks(e) {
  const near = Math.abs(e.clientX - lastDown.x) <= 4 && Math.abs(e.clientY - lastDown.y) <= 4
  lastDown.clicks = near && e.timeStamp - lastDown.time <= 500 ? lastDown.clicks + 1 : 1
  lastDown.time = e.timeStamp
  lastDown.x = e.clientX
  lastDown.y = e.clientY
  return lastDown.clicks
}

scroll.addEventListener('pointerdown', (e) => {
  hook.pointer()
  if (e.button !== 0) return
  const b = scroll.getBoundingClientRect()
  if (e.clientX >= b.left + scroll.clientWidth || e.clientY >= b.top + scroll.clientHeight) return
  e.preventDefault()
  focus()
  const c = pointToCell(e)
  if (e.clientX - b.left < GW) {
    if (e.clientX - b.left > GW - FOLD_W) quiet({ t: 'toggle_fold', row: c.line })
    else quiet({ t: 'toggle_breakpoint', row: c.line })
    return
  }
  const clicks = countClicks(e)
  if (clicks === 1 && !e.shiftKey && inSelection(e)) {
    moving = { x: e.clientX, y: e.clientY, cell: c, active: false }
    scroll.setPointerCapture(e.pointerId)
    return
  }
  if (clicks === 2) send({ t: 'word', ...c })
  else if (clicks >= 3) send({ t: 'select_line', line: c.line })
  else {
    send({ t: 'click', ...c, ext: e.shiftKey })
    if (e.ctrlKey) {
      hook.ctrlClick()
      return
    }
    dragging = true
    scroll.setPointerCapture(e.pointerId)
  }
})

let hoverTimer = 0
scroll.addEventListener('pointermove', (e) => {
  if (moving) {
    if (!moving.active && Math.hypot(e.clientX - moving.x, e.clientY - moving.y) < 4) return
    moving.active = true
    scroll.classList.add('moving')
    showDrop(e)
    if (!edgeTimer) edgeTimer = setTimeout(edgeScroll, 40)
    return
  }
  if (dragging) {
    dragTo(e)
    if (!edgeTimer) edgeTimer = setTimeout(edgeScroll, 40)
    return
  }
  clearTimeout(hoverTimer)
  hook.hoverEnd(e)
  const b = scroll.getBoundingClientRect()
  if (e.clientX - b.left < GW) return
  const c = pointToCell(e)
  const at = { x: e.clientX, y: e.clientY }
  hoverTimer = setTimeout(() => hook.hover(c, at), 500)
})
scroll.addEventListener('pointerleave', () => clearTimeout(hoverTimer))

const stopDrag = () => {
  dragging = false
}
scroll.addEventListener('pointerup', (e) => {
  stopDrag()
  if (!moving) return
  const { active, cell } = moving
  endMove()
  if (active) {
    send({ t: 'move_selection', ...pointToCell(e), copy: e.ctrlKey })
    return
  }
  send({ t: 'click', ...cell, ext: false })
  if (e.ctrlKey) hook.ctrlClick()
})
scroll.addEventListener('pointercancel', () => {
  stopDrag()
  endMove()
})

scroll.addEventListener(
  'wheel',
  (e) => {
    if (!e.ctrlKey) return
    e.preventDefault()
    const size = Math.max(10, Math.min(24, theme.look.edSize + (e.deltaY < 0 ? 1 : -1)))
    if (size === theme.look.edSize) return
    theme.look.edSize = size
    theme.save()
    theme.apply()
    measure()
  },
  { passive: false },
)

scroll.addEventListener('contextmenu', (e) => {
  e.preventDefault()
  const b = scroll.getBoundingClientRect()
  const c = pointToCell(e)
  if (e.clientX - b.left < GW) {
    hook.gutterContext(c, e)
    return
  }
  if (!view || !view.sel.length) send({ t: 'click', ...c, ext: false })
  focus()
  hook.context(c, e)
})

const mv = (to, ext = false) => ({ t: 'move', to, ext, rows: rows() })
const BINDINGS = {
  ArrowLeft: mv('left'), ArrowRight: mv('right'), ArrowUp: mv('up'), ArrowDown: mv('down'),
  Home: mv('home'), End: mv('end'), PageUp: mv('page_up'), PageDown: mv('page_down'),
  'S-ArrowLeft': mv('left', true), 'S-ArrowRight': mv('right', true), 'S-ArrowUp': mv('up', true), 'S-ArrowDown': mv('down', true),
  'S-Home': mv('home', true), 'S-End': mv('end', true), 'S-PageUp': mv('page_up', true), 'S-PageDown': mv('page_down', true),
  'C-ArrowLeft': mv('word_left'), 'C-ArrowRight': mv('word_right'),
  'C-S-ArrowLeft': mv('word_left', true), 'C-S-ArrowRight': mv('word_right', true),
  'C-Home': mv('doc_start'), 'C-End': mv('doc_end'), 'C-S-Home': mv('doc_start', true), 'C-S-End': mv('doc_end', true),
  Enter: { t: 'newline' }, Backspace: { t: 'backspace' }, Delete: { t: 'delete' },
  'S-Backspace': { t: 'backspace' },
  'C-Backspace': { t: 'delete_word_left' }, 'C-Delete': { t: 'delete_word_right' },
  Tab: { t: 'tab' }, 'S-Tab': { t: 'outdent' },
}

ime.addEventListener('keydown', (e) => {
  if (e.isComposing) return
  const name = keyName(e)
  if (hook.beforeKey(e, name)) {
    e.preventDefault()
    return
  }
  if (name === 'Escape' && moving) {
    e.preventDefault()
    endMove()
    return
  }
  if (name === 'C-ArrowUp' || name === 'C-ArrowDown') {
    e.preventDefault()
    scroll.scrollTop += name === 'C-ArrowUp' ? -LH : LH
    return
  }
  if (name === 'C-x') {
    e.preventDefault()
    cutToClipboard()
    return
  }
  const b = BINDINGS[name]
  if (b) {
    e.preventDefault()
    send(b)
    hook.afterType(name)
  }
})

let composing = false
ime.addEventListener('compositionstart', () => (composing = true))
ime.addEventListener('compositionend', () => {
  composing = false
  flushInput()
})
ime.addEventListener('input', () => {
  if (!composing) flushInput()
})

function flushInput() {
  const s = ime.value
  ime.value = ''
  if (!s) return
  send({ t: 'type', s })
  hook.afterType(s)
  hook.typed(s)
}

ime.addEventListener('copy', (e) => {
  e.preventDefault()
  e.clipboardData.setData('text/plain', clip)
})
ime.addEventListener('paste', (e) => {
  e.preventDefault()
  const s = e.clipboardData.getData('text/plain')
  if (s) send({ t: 'insert', s })
})

export async function copyToClipboard() {
  await idle()
  return invoke('clipboard_write', { text: clip })
}

export async function cutToClipboard() {
  await copyToClipboard()
  send({ t: 'cut_selection' })
}

export async function pasteFromClipboard() {
  const s = await invoke('clipboard_read')
  if (s) send({ t: 'insert', s })
}

export function docCommand(id, cmds) {
  if (id === docId) return quiet(...cmds)
  return invoke('doc_apply', { id, cmds, top: 0, rows: 1, reveal: false })
}
