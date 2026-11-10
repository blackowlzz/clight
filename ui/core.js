export const { invoke } = window.__TAURI__.core
export const { listen } = window.__TAURI__.event
export const win = window.__TAURI__.window.getCurrentWindow()
export const $ = (id) => document.getElementById(id)
export const root = document.documentElement
export const app = {}

export function h(tag, props = {}, ...kids) {
  const el = document.createElement(tag)
  for (const [k, v] of Object.entries(props)) {
    if (k.startsWith('on')) el.addEventListener(k.slice(2), v)
    else if (k === 'class') el.className = v
    else if (k === 'value') el.value = v
    else if (k === 'checked') el.checked = v
    else if (v !== undefined && v !== false) el.setAttribute(k, v === true ? '' : v)
  }
  el.append(...kids.filter((k) => k !== null && k !== undefined && k !== false))
  return el
}

export function icon(name) {
  const s = document.createElementNS('http://www.w3.org/2000/svg', 'svg')
  s.setAttribute('class', 'ic')
  const u = document.createElementNS('http://www.w3.org/2000/svg', 'use')
  u.setAttribute('href', '#i-' + name)
  s.append(u)
  return s
}

export function fileIcon(name, dir = false) {
  const ext = name.includes('.') ? name.slice(name.lastIndexOf('.') + 1).toLowerCase() : ''
  let kind = 'file'
  if (dir) kind = 'folder'
  else if (name === 'CMakeLists.txt' || ext === 'cmake') kind = 'cmake'
  else if (ext === 'c') kind = 'c'
  else if (['cc', 'cpp', 'cxx'].includes(ext)) kind = 'cpp'
  else if (['h', 'hh', 'hpp', 'hxx'].includes(ext)) kind = 'h'
  else if (['md', 'txt'].includes(ext)) kind = 'text'
  const s = icon('x')
  s.firstChild.setAttribute('href', '#f-' + kind)
  s.classList.add('fi', 'fi-' + kind)
  return s
}

export const iconBtn = (name, title, onclick, cls = 'tiny') => h('button', { class: `iconbtn ghost ${cls}`, title, onclick }, icon(name))

export function load(key, fallback) {
  try {
    const v = localStorage.getItem('clight.' + key)
    return v === null ? fallback : JSON.parse(v)
  } catch {
    return fallback
  }
}

export function store(key, v) {
  try {
    localStorage.setItem('clight.' + key, JSON.stringify(v))
  } catch {}
}

export const settings = {
  intel: load('intel', 'auto'),
  index: load('index', 'auto'),
  autosave: load('autosave', true),
  updates: load('updates', true),
  termBar: load('termBar', false),
  formatStyle: load('formatStyle', 'clean'),
}

export const state = {
  root: null,
  build: null,
  profile: 'Debug',
  sys: { cores: 1, total_mb: 0, avail_mb: 0, home: '/' },
  tabs: [],
  active: null,
  files: null,
  targets: [],
  running: false,
  lsp: 'off',
  bps: new Map(),
  conds: new Map(),
  git: null,
  left: 'project',
  bottom: null,
}

export const SEP = navigator.platform.startsWith('Win') ? '\\' : '/'
export const join = (a, b) => (a.endsWith(SEP) ? a + b : a + SEP + b)
export const dirname = (p) => p.slice(0, Math.max(p.lastIndexOf(SEP), 1))
export const basename = (p) => p.slice(p.lastIndexOf(SEP) + 1)
export const activeTab = () => state.tabs.find((t) => t.id === state.active)

let toastTimer = 0
export function toast(msg) {
  const t = $('toast')
  t.textContent = String(msg)
  t.hidden = false
  clearTimeout(toastTimer)
  toastTimer = setTimeout(() => (t.hidden = true), 3600)
}
export const guard = (p) => Promise.resolve(p).catch(toast)

export const actions = new Map()

export function keyName(e) {
  const k = e.key.length === 1 ? e.key.toLowerCase() : e.key
  return (e.ctrlKey || e.metaKey ? 'C-' : '') + (e.altKey ? 'A-' : '') + (e.shiftKey ? 'S-' : '') + k
}

export function keyLabel(k) {
  if (!k) return ''
  return k
    .replace('C-', 'Ctrl+')
    .replace('A-', 'Alt+')
    .replace('S-', 'Shift+')
    .replace(/\+([a-z])$/, (_, c) => '+' + c.toUpperCase())
    .replace('ArrowUp', '↑')
    .replace('ArrowDown', '↓')
    .replace('ArrowLeft', '←')
    .replace('ArrowRight', '→')
}

export function reg(id, title, key, run, opts = {}) {
  actions.set(id, { id, title, key, run, ...opts })
}

export function runAction(id) {
  const a = actions.get(id)
  if (a) guard(a.run())
}

let closeTop = null
export const modalOpen = () => closeTop !== null
export const closeModal = () => closeTop?.()

export function modal(content) {
  const scrim = h('div', { class: 'scrim' }, content)
  const close = () => {
    scrim.remove()
    closeTop = null
    app.refocus?.()
  }
  scrim.addEventListener('pointerdown', (e) => e.target === scrim && close())
  $('modal-root').replaceChildren(scrim)
  closeTop = close
  return close
}

export function confirmBox(title, body, yes, no = 'Cancel') {
  return new Promise((resolve) => {
    let close
    const done = (v) => {
      close()
      resolve(v)
    }
    const ok = h('button', { class: 'primary', onclick: () => done(true) }, yes)
    close = modal(h('div', { class: 'dialog' }, h('h3', {}, title), h('p', { class: 'muted' }, body), h('div', { class: 'actions' }, h('button', { onclick: () => done(false) }, no), ok)))
    ok.focus()
  })
}

export function promptBox(title, value, action) {
  return new Promise((resolve) => {
    let close
    const input = h('input', { value, spellcheck: 'false' })
    const done = (v) => {
      close()
      resolve(v)
    }
    input.addEventListener('keydown', (e) => e.key === 'Enter' && input.value && done(input.value))
    close = modal(h('div', { class: 'dialog' }, h('h3', {}, title), input, h('div', { class: 'actions' }, h('button', { onclick: () => done(null) }, 'Cancel'), h('button', { class: 'primary', onclick: () => input.value && done(input.value) }, action))))
    input.focus()
    input.select()
  })
}

export function palette({ placeholder, query, render, debounce = 0 }) {
  const input = h('input', { placeholder, spellcheck: 'false' })
  const results = h('div', { class: 'results' })
  let items = []
  let sel = 0
  let token = 0
  let timer = 0
  const close = modal(h('div', { class: 'palette' }, input, results))
  const paint = () => {
    results.replaceChildren(
      ...items.map((it, i) => {
        const row = render(it)
        row.classList.add('res')
        if (i === sel) row.classList.add('on')
        row.addEventListener('pointerdown', (e) => {
          e.preventDefault()
          close()
          it.open()
        })
        return row
      }),
    )
    results.children[sel]?.scrollIntoView({ block: 'nearest' })
  }
  const update = async () => {
    const t = ++token
    const r = await query(input.value)
    if (t !== token) return
    items = r
    sel = 0
    paint()
  }
  input.addEventListener('input', () => {
    clearTimeout(timer)
    timer = setTimeout(update, debounce)
  })
  input.addEventListener('keydown', (e) => {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault()
      if (items.length) {
        sel = (sel + (e.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length
        paint()
      }
    } else if (e.key === 'Enter' && items[sel]) {
      close()
      items[sel].open()
    }
  })
  input.focus()
  update()
  return input
}

export function fuzzy(q, s) {
  const ql = q.toLowerCase(), sl = s.toLowerCase()
  const nameStart = sl.lastIndexOf(SEP) + 1
  let i = 0, score = 0, prev = -2
  for (let j = 0; j < sl.length && i < ql.length; j++) {
    if (sl[j] === ql[i]) {
      score += j >= nameStart ? 3 : 1
      if (j === prev + 1) score += 2
      if (j === nameStart) score += 6
      prev = j
      i++
    }
  }
  return i === ql.length ? score - s.length * 0.01 : -1
}

export function seg(options, value, pick) {
  const el = h('div', { class: 'seg' })
  const paint = (cur) => el.replaceChildren(...options.map(([v, label]) => h('button', { 'aria-pressed': String(v === cur), onclick: () => (pick(v), paint(v)) }, label)))
  paint(value)
  return el
}

let menuClose = null

export function showMenu(x, y, items, onClose) {
  const m = $('menu')
  menuClose?.()
  m.replaceChildren(
    ...items.map((it) =>
      it === '-'
        ? h('hr')
        : h('button', { disabled: it[3] === false, onclick: () => (hideMenu(), it[1]()) }, h('span', {}, it[0]), it[2] ? h('span', {}, it[2]) : null),
    ),
  )
  m.hidden = false
  menuClose = onClose ?? null
  m.style.left = Math.max(4, Math.min(x, innerWidth - m.offsetWidth - 6)) + 'px'
  m.style.top = Math.max(4, Math.min(y, innerHeight - m.offsetHeight - 6)) + 'px'
}

export function hideMenu() {
  $('menu').hidden = true
  const c = menuClose
  menuClose = null
  c?.()
}
export const menuVisible = () => !$('menu').hidden

$('menu').addEventListener('mousedown', (e) => e.preventDefault())

document.addEventListener('pointerdown', (e) => {
  if (!e.target.closest('#menu') && !e.target.closest('#menus')) hideMenu()
})
