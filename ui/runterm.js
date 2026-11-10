import * as ed from './editor.js'
import { $, h, invoke, listen, keyName } from './core.js'
import { termSequence, TERMINAL_PASS } from './termkeys.js'

const ID = 0

const view = $('run-term')
const pipes = $('run-out')
const cursorEl = h('div', { class: 'tcursor' })
const input = h('textarea', { class: 'tinput', autocapitalize: 'off', spellcheck: 'false' })
view.append(cursorEl, input)

let rowEls = []
let cols = 0, rows = 0
let offset = 0, scrollback = 0
let appCursor = false, bracketed = false
let running = false, used = false
let pulling = false, again = false
let finish = null

function size() {
  const { LH, CH } = ed.metrics()
  return { c: Math.max(10, Math.floor(view.clientWidth / CH) - 1), r: Math.max(2, Math.floor(view.clientHeight / LH)) }
}

function layoutRows(r) {
  const { LH } = ed.metrics()
  while (rowEls.length > r) rowEls.pop().remove()
  while (rowEls.length < r) {
    const el = h('div', { class: 'tr' })
    el.style.top = rowEls.length * LH + 'px'
    view.insertBefore(el, cursorEl)
    rowEls.push(el)
  }
}

async function pull(full = false) {
  if (!used || view.clientHeight === 0) return
  if (pulling) {
    again = true
    return
  }
  pulling = true
  try {
    const v = await invoke('term_view', { id: ID, offset, full })
    for (const [y, html] of v.rows) {
      if (rowEls[y] && rowEls[y]._h !== html) {
        rowEls[y]._h = html
        rowEls[y].innerHTML = html
      }
    }
    scrollback = v.scrollback
    appCursor = v.app_cursor
    bracketed = v.bracketed
    const { LH, CH } = ed.metrics()
    cursorEl.hidden = !running || !v.cursor || offset > 0
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
  if (e.payload !== ID || frameQueued) return
  frameQueued = true
  requestAnimationFrame(() => {
    frameQueued = false
    pull()
  })
})

listen('run-exit', (e) => {
  running = false
  const done = finish
  finish = null
  done?.(e.payload)
})

function focus() {
  input.focus({ preventScroll: true })
}

export const hasFocus = () => document.activeElement === input
export const inUse = () => used

function showTerminal(on) {
  view.hidden = !on
  pipes.hidden = on
}

export function usePipes() {
  used = false
  showTerminal(false)
}

export function start({ program, args, cwd, env, banner }) {
  used = true
  running = true
  offset = 0
  showTerminal(true)
  const { c, r } = size()
  cols = c
  rows = r
  layoutRows(r)
  for (const el of rowEls) {
    el._h = ''
    el.innerHTML = ''
  }
  return new Promise((resolve, reject) => {
    finish = resolve
    invoke('run_start', { program, args, cwd, env, cols: c, rows: r, banner })
      .then(() => {
        pull(true)
        focus()
      })
      .catch((e) => {
        finish = null
        running = false
        reject(e)
      })
  })
}

export async function note(text) {
  await invoke('term_note', { id: ID, text }).catch(() => {})
  offset = 0
  await pull(true)
}

export function clear() {
  if (!running) note('\x1b[H\x1b[2J\x1b[3J')
}

export function shown() {
  if (!used) return
  applySize()
  pull(true)
}

function applySize() {
  if (!used || view.clientHeight === 0) return
  const { c, r } = size()
  if (c === cols && r === rows) return
  cols = c
  rows = r
  layoutRows(r)
  invoke('term_resize', { id: ID, cols: c, rows: r }).then(() => pull(true))
}

let resizeTimer = 0
new ResizeObserver(() => {
  clearTimeout(resizeTimer)
  resizeTimer = setTimeout(applySize, 80)
}).observe(view)

function send(data) {
  if (!running) return
  if (offset) {
    offset = 0
    pull(true)
  }
  invoke('term_input', { id: ID, data }).catch(() => {})
}

input.addEventListener('keydown', (e) => {
  if (e.isComposing) return
  const name = keyName(e)
  if (name === 'C-S-c' || (name === 'C-c' && getSelection().toString())) {
    e.preventDefault()
    document.execCommand('copy')
    return
  }
  if (name === 'C-S-v' || name === 'C-v' || TERMINAL_PASS.has(name)) return
  const seq = termSequence(e, name, appCursor)
  if (seq !== null) {
    e.preventDefault()
    send(seq)
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
  if (text) send(bracketed ? `\x1b[200~${text}\x1b[201~` : text)
})

view.addEventListener('pointerup', () => {
  if (!getSelection().toString()) focus()
})

view.addEventListener(
  'wheel',
  (e) => {
    if (!used) return
    offset = Math.max(0, Math.min(scrollback, offset + (e.deltaY < 0 ? 3 : -3)))
    pull(true)
  },
  { passive: true },
)
