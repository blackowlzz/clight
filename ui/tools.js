import { h, invoke, listen, app, state, load, store, reg, toast, guard, modal, SEP } from './core.js'

const LABEL = 'Download clangd utils'
const DISMISSED = 'tools-dismissed'

let note = null
let noteText = null
let busy = false
const bars = new Map()
const titles = new Map()

const size = (n) => {
  const mb = n / 1048576
  return `${mb < 10 ? mb.toFixed(1) : Math.round(mb)} MB`
}

function hideNote() {
  note?.remove()
  note = null
}

function showNote(text = LABEL) {
  if (!note) {
    noteText = h('button', { class: 'tn-open', onclick: () => guard(openMenu()) })
    const dismiss = h('button', {
      class: 'tn-x',
      title: 'Dismiss',
      onclick: () => {
        if (busy) return
        store(DISMISSED, true)
        hideNote()
      },
    }, '×')
    note = h('div', { id: 'tools-note' }, noteText, dismiss)
    document.body.append(note)
  }
  noteText.textContent = text
}

async function install(ids, close) {
  busy = true
  showNote('Downloading…')
  try {
    const failures = await invoke('tools_install', { ids })
    close()
    hideNote()
    if (failures.length) toast(`${failures[0].title}: ${failures[0].error}`)
    else toast('Tools installed')
    if (state.root && state.lsp !== 'off') app.restartIntel?.()
  } finally {
    busy = false
    bars.clear()
  }
}

async function openMenu() {
  if (busy) return toast('Already downloading')
  const list = await invoke('tools_status')
  list.forEach((t) => titles.set(t.id, t.title))
  const boxes = new Map()
  const total = h('span', { class: 'muted' })
  const go = h('button', { class: 'primary' }, 'Install')
  const refresh = () => {
    const picked = list.filter((t) => boxes.get(t.id)?.checked)
    total.textContent = picked.length ? `${size(picked.reduce((n, t) => n + t.size, 0))} to download` : 'Nothing selected'
    go.disabled = !picked.length
  }
  const rows = list.map((t) => {
    const box = h('input', { type: 'checkbox', checked: t.recommended && !t.installed, disabled: t.installed, onchange: refresh })
    boxes.set(t.id, box)
    const state_ = h('span', { class: 'tool-size' }, t.installed ? 'Installed' : size(t.size))
    bars.set(t.id, state_)
    return h('label', { class: 'tool-row' + (t.installed ? ' done' : '') }, box, h('span', {}, h('b', {}, t.title, t.recommended ? h('i', {}, ' recommended') : null), h('span', { class: 'small' }, t.detail)), state_)
  })
  let close
  const later = h('button', { onclick: () => close() }, 'Later')
  go.addEventListener('click', () => {
    const ids = list.filter((t) => boxes.get(t.id).checked).map((t) => t.id)
    go.disabled = true
    later.textContent = 'Hide'
    boxes.forEach((b) => (b.disabled = true))
    guard(install(ids, () => close()))
  })
  close = modal(h('div', { class: 'dialog' }, h('h3', {}, LABEL), h('p', { class: 'muted' }, 'Everything goes in clight’s own folder and your system stays as it is.'), h('div', { class: 'tool-list' }, ...rows), h('div', { class: 'actions tool-actions' }, total, later, go)))
  refresh()
}

listen('tools-progress', (e) => {
  const { id, done, total } = e.payload
  const pct = Math.min(100, Math.floor((done / total) * 100))
  const el = bars.get(id)
  if (el) el.textContent = `${pct}%`
  if (busy) showNote(`Downloading ${titles.get(id) ?? id} ${pct}%`)
})

reg('download-tools', `${LABEL}…`, null, openMenu, { hidden: SEP !== '\\' })

export async function startup() {
  const list = await invoke('tools_status')
  if (list.some((t) => t.recommended && !t.installed) && !load(DISMISSED, false)) showNote()
}
