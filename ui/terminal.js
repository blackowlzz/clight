import * as ed from './editor.js'
import { $, h, icon, invoke, listen, state, settings, load, store, keyName, toast } from './core.js'

const view = $('v-terminal')
const termEl = h('div', { id: 'term' })
const cursorEl = h('div', { id: 'term-cursor' })
const input = h('textarea', { id: 'term-input', autocapitalize: 'off', spellcheck: 'false' })
termEl.append(cursorEl, input)

const cmdInput = h('input', { id: 'term-cmd', spellcheck: 'false', autocomplete: 'off', autocapitalize: 'off', placeholder: 'Type a command' })
const pathEl = h('span', { class: 'tb-path' })
const branchEl = h('span', { class: 'tb-branch' })
const hintEl = h('div', { class: 'tb-hint' })
const bar = h('div', { id: 'term-bar' }, h('div', { class: 'tb-info' }, pathEl, branchEl), hintEl, h('div', { class: 'tb-line' }, h('span', { class: 'tb-mark' }, '›'), cmdInput))
hintEl.hidden = true
view.replaceChildren(termEl, bar)

const history = load('termHistory', [])
let historyPos = history.length
let draft = ''
let cwd = ''
let infoTimer = 0

const barActive = () => settings.termBar && !cur()?.alt

export function syncBar() {
  bar.hidden = !barActive()
  cursorEl.hidden = barActive() || !cur()
}

function scheduleInfo() {
  clearTimeout(infoTimer)
  infoTimer = setTimeout(async () => {
    const t = cur()
    if (!t || !barActive()) return
    const info = await invoke('term_info', { id: t.id }).catch(() => null)
    if (!info) return
    cwd = info.cwd
    const home = state.sys.home
    pathEl.textContent = home && cwd.startsWith(home) ? '~' + cwd.slice(home.length) : cwd
    branchEl.textContent = info.branch ?? ''
    cmdInput.placeholder = info.busy ? 'Send input to the running program' : 'Type a command'
  }, 200)
}

const tabsEl = h('span', { class: 'termtabs' })
export const panelActions = () => tabsEl

let tabs = []
let active = null
let rowEls = []
let cols = 0, rows = 0
let pulling = false, again = false

const cur = () => tabs.find((t) => t.id === active)

function size() {
  const { LH, CH } = ed.metrics()
  return { c: Math.max(10, Math.floor(termEl.clientWidth / CH) - 1), r: Math.max(2, Math.floor(termEl.clientHeight / LH)) }
}

function layoutRows(r) {
  const { LH } = ed.metrics()
  while (rowEls.length > r) rowEls.pop().remove()
  while (rowEls.length < r) {
    const el = h('div', { class: 'tr' })
    el.style.top = rowEls.length * LH + 'px'
    termEl.insertBefore(el, cursorEl)
    rowEls.push(el)
  }
}

async function pull(full = false) {
  const t = cur()
  if (!t || view.hidden) return
  if (pulling) {
    again = true
    return
  }
  pulling = true
  try {
    const v = await invoke('term_view', { id: t.id, offset: t.offset, full })
    for (const [y, html] of v.rows) {
      if (rowEls[y] && rowEls[y]._h !== html) {
        rowEls[y]._h = html
        rowEls[y].innerHTML = html
      }
    }
    t.scrollback = v.scrollback
    t.app = v.app_cursor
    t.bracketed = v.bracketed
    if (t.alt !== v.alt) {
      t.alt = v.alt
      syncBar()
      if (hadFocus()) focus()
    } else t.alt = v.alt
    scheduleInfo()
    const { LH, CH } = ed.metrics()
    cursorEl.hidden = barActive() || !v.cursor || t.offset > 0
    cursorEl.style.transform = `translate(${v.x * CH}px,${v.y * LH}px)`
    cursorEl.style.width = CH + 'px'
    input.style.top = v.y * LH + 'px'
  } catch {}
  pulling = false
  if (again) {
    again = false
    pull()
  }
}

let frameQueued = false
listen('term-dirty', (e) => {
  if (e.payload !== active || frameQueued) return
  frameQueued = true
  requestAnimationFrame(() => {
    frameQueued = false
    pull()
  })
})

listen('term-exit', (e) => closeTab(e.payload))

function renderTabs() {
  tabsEl.replaceChildren(
    ...tabs.map((t, i) => h('button', { class: 'toggle', 'aria-pressed': String(t.id === active), onclick: () => switchTo(t.id), onauxclick: (ev) => ev.button === 1 && close(t.id) }, `${i + 1}`)),
    h('button', { class: 'iconbtn ghost tiny', title: 'New terminal', onclick: () => open() }, icon('plus')),
    tabs.length ? h('button', { class: 'iconbtn ghost tiny', title: 'Close terminal', onclick: () => close(active) }, icon('stop')) : null,
  )
}

function switchTo(id) {
  active = id
  renderTabs()
  applySize()
  pull(true)
  focus()
}

function close(id) {
  invoke('term_close', { id })
}

function closeTab(id) {
  const i = tabs.findIndex((t) => t.id === id)
  if (i < 0) return
  tabs.splice(i, 1)
  if (active === id) {
    active = tabs[Math.min(i, tabs.length - 1)]?.id ?? null
    if (active !== null) pull(true)
    else {
      rowEls.forEach((r) => {
        r._h = ''
        r.innerHTML = ''
      })
      cursorEl.hidden = true
    }
  }
  renderTabs()
}

async function open() {
  const { c, r } = size()
  cols = c
  rows = r
  layoutRows(r)
  try {
    const id = await invoke('term_open', { cwd: state.root ?? state.sys.home, cols: c, rows: r })
    tabs.push({ id, offset: 0, scrollback: 0, app: false, bracketed: false, alt: false })
    active = id
    renderTabs()
    pull(true)
    focus()
  } catch (e) {
    toast(e)
  }
}

function applySize() {
  if (view.hidden) return
  const { c, r } = size()
  if (c === cols && r === rows) return
  cols = c
  rows = r
  layoutRows(r)
  for (const t of tabs) invoke('term_resize', { id: t.id, cols: c, rows: r })
  pull(true)
}

let resizeTimer = 0
new ResizeObserver(() => {
  clearTimeout(resizeTimer)
  resizeTimer = setTimeout(applySize, 80)
}).observe(termEl)

export function shown() {
  syncBar()
  if (!tabs.length) open()
  else {
    applySize()
    pull(true)
    focus()
    scheduleInfo()
  }
}

export function closeAll() {
  for (const t of tabs) invoke('term_close', { id: t.id })
}

export function focus() {
  ;(barActive() ? cmdInput : input).focus({ preventScroll: true })
}

const hadFocus = () => document.activeElement === input || document.activeElement === cmdInput
export const hasFocus = hadFocus

function rememberCommand(text) {
  if (!text.trim() || history[history.length - 1] === text) return
  history.push(text)
  if (history.length > 200) history.shift()
  store('termHistory', history)
}

function browseHistory(delta) {
  if (historyPos === history.length) draft = cmdInput.value
  historyPos = Math.max(0, Math.min(history.length, historyPos + delta))
  cmdInput.value = historyPos === history.length ? draft : history[historyPos]
  cmdInput.setSelectionRange(cmdInput.value.length, cmdInput.value.length)
}

async function completePath() {
  const value = cmdInput.value
  const start = value.lastIndexOf(' ') + 1
  const word = value.slice(start)
  const expanded = word.startsWith('~') ? state.sys.home + word.slice(1) : word
  const cut = expanded.lastIndexOf('/') + 1
  const dirPart = expanded.slice(0, cut)
  const prefix = expanded.slice(cut)
  const dir = dirPart.startsWith('/') ? dirPart : `${cwd}/${dirPart}`
  const entries = await invoke('list_dir', { path: dir, hidden: prefix.startsWith('.') }).catch(() => [])
  const matches = entries.filter((e) => e.name.startsWith(prefix))
  if (!matches.length) return
  let common = matches[0].name
  for (const m of matches) while (!m.name.startsWith(common)) common = common.slice(0, -1)
  const suffix = matches.length === 1 ? (matches[0].dir ? '/' : ' ') : ''
  cmdInput.value = value.slice(0, start) + word.slice(0, word.length - prefix.length) + common + suffix
  hintEl.textContent = matches.length > 1 ? matches.slice(0, 40).map((m) => m.name).join('   ') : ''
  hintEl.hidden = matches.length === 1
}

const CONTROL = { 'C-d': '\x04', 'C-z': '\x1a', 'C-l': '\x0c' }

cmdInput.addEventListener('keydown', (e) => {
  const name = keyName(e)
  if (name === 'Enter') {
    e.preventDefault()
    rememberCommand(cmdInput.value)
    send(cmdInput.value + '\r')
    cmdInput.value = ''
    draft = ''
    historyPos = history.length
    hintEl.hidden = true
    scheduleInfo()
  } else if (name === 'ArrowUp' || name === 'ArrowDown') {
    e.preventDefault()
    browseHistory(name === 'ArrowUp' ? -1 : 1)
  } else if (name === 'Tab') {
    e.preventDefault()
    completePath()
  } else if (name === 'C-c' && cmdInput.selectionStart === cmdInput.selectionEnd) {
    e.preventDefault()
    send('\x03')
    cmdInput.value = ''
  } else if (CONTROL[name] && (name !== 'C-d' || !cmdInput.value)) {
    e.preventDefault()
    send(CONTROL[name])
  }
})

function send(data) {
  const t = cur()
  if (!t) return
  if (t.offset) {
    t.offset = 0
    pull(true)
  }
  invoke('term_input', { id: t.id, data })
}

const SPECIAL = {
  Enter: '\r', Backspace: '\x7f', Tab: '\t', Escape: '\x1b', 'S-Tab': '\x1b[Z',
  Home: '\x1b[H', End: '\x1b[F', PageUp: '\x1b[5~', PageDown: '\x1b[6~', Delete: '\x1b[3~', Insert: '\x1b[2~',
  F1: '\x1bOP', F2: '\x1bOQ', F3: '\x1bOR', F4: '\x1bOS', F5: '\x1b[15~', F6: '\x1b[17~', F7: '\x1b[18~', F8: '\x1b[19~', F9: '\x1b[20~', F10: '\x1b[21~', F11: '\x1b[23~', F12: '\x1b[24~',
}
const ARROW = { ArrowUp: 'A', ArrowDown: 'B', ArrowRight: 'C', ArrowLeft: 'D' }

input.addEventListener('keydown', (e) => {
  if (e.isComposing) return
  const name = keyName(e)
  if (name === 'C-S-c') {
    e.preventDefault()
    document.execCommand('copy')
    return
  }
  if (name === 'C-S-v') return
  if (name === 'A-F12') return

  if (ARROW[e.key]) {
    e.preventDefault()
    const mod = 1 + (e.shiftKey ? 1 : 0) + (e.altKey ? 2 : 0) + (e.ctrlKey ? 4 : 0)
    send(mod > 1 ? `\x1b[1;${mod}${ARROW[e.key]}` : `\x1b${cur()?.app ? 'O' : '['}${ARROW[e.key]}`)
    return
  }
  if (SPECIAL[name] ?? (e.shiftKey ? undefined : SPECIAL[e.key])) {
    e.preventDefault()
    send(SPECIAL[name] ?? SPECIAL[e.key])
    return
  }
  if (e.ctrlKey && !e.altKey && e.key.length === 1) {
    const k = e.key.toLowerCase()
    const code = k >= 'a' && k <= 'z' ? k.charCodeAt(0) - 96 : { '[': 27, '\\': 28, ']': 29, '^': 30, _: 31, ' ': 0 }[k]
    if (code !== undefined) {
      e.preventDefault()
      send(String.fromCharCode(code))
    }
    return
  }
  if (e.altKey && !e.ctrlKey && e.key.length === 1) {
    e.preventDefault()
    send('\x1b' + e.key)
  }
})

input.addEventListener('input', () => {
  if (input.value) {
    send(input.value)
    input.value = ''
  }
})

input.addEventListener('paste', (e) => {
  e.preventDefault()
  const text = e.clipboardData.getData('text/plain')
  if (text) send(cur()?.bracketed ? `\x1b[200~${text}\x1b[201~` : text)
})

termEl.addEventListener('pointerup', () => {
  if (!getSelection().toString()) focus()
})

bar.addEventListener('pointerup', () => focus())

termEl.addEventListener(
  'wheel',
  (e) => {
    const t = cur()
    if (!t) return
    if (t.alt) {
      send(`\x1b${t.app ? 'O' : '['}${e.deltaY < 0 ? 'A' : 'B'}`.repeat(3))
      return
    }
    t.offset = Math.max(0, Math.min(t.scrollback, t.offset + (e.deltaY < 0 ? 3 : -3)))
    pull(true)
  },
  { passive: true },
)
