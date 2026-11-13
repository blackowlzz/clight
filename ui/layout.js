// Where each tool window lives, as in VS Code: the side bar on the left, the one on the right,
// or the panel under the editor. A view moves by dragging its button in the rail (or the title
// of the place it is shown in) onto another place or another spot in the rail, or from the
// button's menu. The layout is remembered.
import { $, h, icon, state, load, store, showMenu } from './core.js'

// The panel and the right side bar show their views as tabs; the left side bar shows the title
// of its view, the rail being the way to change it (as VS Code does).
const PLACES = {
  left: { box: 'left', rz: 'side-rz', head: 'left', name: 'Left Side Bar', tabs: false },
  right: { box: 'right', rz: 'right-rz', head: 'right', name: 'Right Side Bar', tabs: true },
  bottom: { box: 'panel', rz: 'panel-rz', head: 'panel', name: 'Panel', tabs: true },
}
const KEY = 'views'

/** name -> { title, icon, tip, home, rail = true, available(), shown(actions, still), click() } */
let views = {}
/** name -> place */
let where = {}
/** Every view, in the order of the rail. */
let order = []
let changed = () => {}

export const isShown = (name) => where[name] !== undefined && state[where[name]] === name
export const placeOf = (name) => where[name]

/** Sets up the views. `onChange` runs whenever a place opens, closes or shows another view. */
export function define(defs, { onChange } = {}) {
  views = defs
  changed = onChange ?? changed
  const saved = load(KEY, null)
  where = {}
  for (const [n, d] of Object.entries(defs)) where[n] = PLACES[saved?.where?.[n]] ? saved.where[n] : d.home
  const known = (saved?.order ?? []).filter((n) => n in defs)
  order = [...known, ...Object.keys(defs).filter((n) => !known.includes(n))]
  for (const p of Object.keys(PLACES)) state[p] = null
  for (const n of Object.keys(defs)) place(n)
  for (const p of Object.keys(PLACES)) {
    $(PLACES[p].box).hidden = true
    $(PLACES[p].rz).hidden = true
    $(PLACES[p].head + '-x')?.addEventListener('click', () => hide(p))
    const title = $(PLACES[p].head + '-title')
    if (PLACES[p].tabs) title.classList.add('ptabs')
    else title.classList.add('grab')
    title.addEventListener('pointerdown', (e) => !e.target.closest('.ptab') && state[p] && startDrag(state[p], e, title))
  }
  renderRail()
}

function place(name) {
  const el = $('v-' + name)
  const box = $(PLACES[where[name]].box)
  if (el.parentElement !== box) box.appendChild(el)
  if (!isShown(name)) el.hidden = true
}

/** Shows a view where it lives. With `toggle`, a view already shown is hidden instead. */
export async function show(name, { toggle = false } = {}) {
  const d = views[name]
  if (!d || d.available?.() === false) return
  const p = where[name]
  if (toggle && state[p] === name) return hide(p)
  const P = PLACES[p]
  state[p] = name
  place(name)
  $(P.box).hidden = false
  $(P.rz).hidden = false
  for (const n of Object.keys(views)) if (where[n] === p) $('v-' + n).hidden = n !== name
  if (P.tabs) renderTabs(p)
  else $(P.head + '-title').textContent = d.title
  $(P.head + '-sub').textContent = ''
  const acts = $(P.head + '-actions')
  acts.replaceChildren()
  markRail()
  changed()
  await d.shown?.(acts, () => state[p] === name)
}

export function hide(p) {
  if (!state[p]) return
  state[p] = null
  $(PLACES[p].box).hidden = true
  $(PLACES[p].rz).hidden = true
  markRail()
  changed()
}

/** Hides `name` if it is shown. */
export function close(name) {
  if (isShown(name)) hide(where[name])
}

/** Changes the title or the line next to it, while `name` is the view shown. */
export function setHeader(name, title, sub) {
  if (!isShown(name)) return
  const P = PLACES[where[name]]
  if (title !== undefined) {
    if (P.tabs) {
      const tab = $(P.head + '-title').querySelector(`[data-view="${name}"] span`)
      if (tab) tab.textContent = title
    } else $(P.head + '-title').textContent = title
  }
  if (sub !== undefined) $(P.head + '-sub').textContent = sub
}

/** Moves a view to `to`, before the `index`-th view there (at the end without one), and shows it. */
export function move(name, to, index) {
  const from = where[name]
  const was = isShown(name)
  where[name] = to
  if (from !== to && was) {
    // The place it leaves stays open with another of its views, as in VS Code; empty, it closes.
    const next = order.find((n) => n !== name && where[n] === from && views[n].available?.() !== false)
    if (next) show(next)
    else hide(from)
  }
  const rest = order.filter((n) => n !== name)
  const there = rest.filter((n) => where[n] === to)
  const before = index === undefined ? undefined : there[index]
  rest.splice(before === undefined ? rest.length : rest.indexOf(before), 0, name)
  order = rest
  save()
  renderRail()
  // Reordering the rail leaves the view as it was; moving it somewhere else shows it there.
  if (from !== to) show(name)
}

export function reset() {
  for (const p of Object.keys(PLACES)) hide(p)
  where = Object.fromEntries(Object.entries(views).map(([n, d]) => [n, d.home]))
  order = Object.keys(views)
  for (const n of order) place(n)
  save()
  renderRail()
}

function save() {
  store(KEY, { where, order })
}

/** Shows or hides the buttons of views that can't be used right now (the chat when it is off). */
export function refreshRail() {
  for (const b of document.querySelectorAll('#rail [data-view]')) b.hidden = views[b.dataset.view]?.available?.() === false
  for (const p of Object.keys(PLACES)) if (PLACES[p].tabs && state[p]) renderTabs(p)
}

/** The tabs of a place: one per view living there. A click shows it, a drag moves it. */
function renderTabs(p) {
  const strip = $(PLACES[p].head + '-title')
  strip.replaceChildren(
    ...order
      .filter((n) => where[n] === p && views[n].available?.() !== false)
      .map((n) => {
        const t = h('button', { class: 'ptab' + (state[p] === n ? ' on' : ''), 'data-view': n, title: views[n].tip ?? views[n].title }, h('span', {}, views[n].title))
        t.addEventListener('pointerdown', (e) => startDrag(n, e, t))
        t.addEventListener('contextmenu', (e) => {
          e.preventDefault()
          showMenu(e.clientX, e.clientY, [
            ...Object.entries(PLACES).map(([q, Q]) => [`Move to ${Q.name}`, () => move(n, q), '', where[n] !== q]),
            '-',
            ['Reset Layout', reset],
          ])
        })
        return t
      }),
  )
}

function renderRail() {
  for (const p of Object.keys(PLACES)) {
    const group = $('rail-' + p)
    group.replaceChildren(...order.filter((n) => where[n] === p && views[n].rail !== false).map(railButton))
    if (PLACES[p].tabs && state[p]) renderTabs(p)
  }
  refreshRail()
  markRail()
}

function railButton(name) {
  const d = views[name]
  const b = h('button', { 'data-view': name, title: d.tip ?? d.title }, icon(d.icon))
  b.addEventListener('pointerdown', (e) => startDrag(name, e, b))
  b.addEventListener('contextmenu', (e) => {
    e.preventDefault()
    showMenu(e.clientX, e.clientY, [
      ...Object.entries(PLACES).map(([p, P]) => [`Move to ${P.name}`, () => move(name, p), '', where[name] !== p]),
      '-',
      ['Reset Layout', reset],
    ])
  })
  return b
}

function markRail() {
  for (const b of document.querySelectorAll('#rail [data-view]')) b.classList.toggle('on', isShown(b.dataset.view))
}

// Dragging. Below a few pixels of movement a press is a click.
let drag = null

function startDrag(name, e, from) {
  if (e.button !== 0) return
  e.preventDefault()
  try {
    from.setPointerCapture(e.pointerId)
  } catch {}
  drag = { name, from, x: e.clientX, y: e.clientY, on: false, target: null }
  from.addEventListener('pointermove', onMove)
  from.addEventListener('pointerup', onUp)
  from.addEventListener('pointercancel', cancel)
  addEventListener('keydown', onKey, true)
}

function onMove(e) {
  if (!drag) return
  if (!drag.on) {
    if (Math.hypot(e.clientX - drag.x, e.clientY - drag.y) < 5) return
    drag.on = true
    const d = views[drag.name]
    drag.ghost = h('div', { class: 'view-ghost' }, icon(d.icon), h('span', {}, d.title))
    drag.zone = h('div', { class: 'drop-zone' })
    drag.line = h('div', { class: 'rail-drop' })
    drag.zone.hidden = drag.line.hidden = true
    document.body.append(drag.zone, drag.line, drag.ghost)
    document.documentElement.classList.add('dragging-view')
  }
  // Beside the pointer, but kept inside the window near its edges.
  const gx = Math.min(e.clientX + 12, innerWidth - drag.ghost.offsetWidth - 6)
  const gy = Math.min(e.clientY + 10, innerHeight - drag.ghost.offsetHeight - 6)
  drag.ghost.style.transform = `translate(${gx}px,${gy}px)`
  drag.target = targetAt(e.clientX, e.clientY)
  paintTarget(drag.target)
}

function onUp(e) {
  if (!drag) return
  const { name, on, target } = drag
  cancel()
  if (!on) {
    const d = views[name]
    const el = e.currentTarget
    if (d.click) d.click()
    else if (el.classList?.contains('ptab')) show(name)
    else if (el.dataset?.view) show(name, { toggle: true })
    return
  }
  if (target) move(name, target.place, target.index)
}

function onKey(e) {
  if (e.key !== 'Escape' || !drag) return
  e.preventDefault()
  e.stopPropagation()
  cancel()
}

function cancel() {
  if (!drag) return
  const { from } = drag
  from.removeEventListener('pointermove', onMove)
  from.removeEventListener('pointerup', onUp)
  from.removeEventListener('pointercancel', cancel)
  removeEventListener('keydown', onKey, true)
  drag.ghost?.remove()
  drag.zone?.remove()
  drag.line?.remove()
  document.documentElement.classList.remove('dragging-view')
  drag = null
}

/** Where a drop at (x, y) would put the view: a spot in the rail, or one of the places. */
function targetAt(x, y) {
  const rail = $('rail').getBoundingClientRect()
  if (x >= rail.left && x <= rail.right + 8) {
    // The nearest gap between buttons, in any of the three groups of the rail.
    let best = null
    for (const p of Object.keys(PLACES)) {
      const group = $('rail-' + p)
      const buttons = [...group.querySelectorAll('[data-view]')].filter((b) => !b.hidden && b.dataset.view !== drag.name)
      const gaps = buttons.map((b, i) => ({ y: b.getBoundingClientRect().top - 1.5, index: i }))
      const end = buttons.length ? buttons[buttons.length - 1].getBoundingClientRect().bottom + 1.5 : group.getBoundingClientRect().top
      gaps.push({ y: end, index: undefined })
      for (const g of gaps) if (!best || Math.abs(g.y - y) < Math.abs(best.y - y)) best = { ...g, place: p }
    }
    return best && { place: best.place, index: best.index, line: best.y, rail }
  }
  // Over the tabs of a place: between two of them.
  for (const [p, P] of Object.entries(PLACES)) {
    if (!P.tabs || $(P.box).hidden) continue
    const strip = $(P.head + '-title').getBoundingClientRect()
    const head = $(P.head + '-title').parentElement.getBoundingClientRect()
    if (y < head.top || y > head.bottom || x < head.left || x > head.right) continue
    const tabs = [...$(P.head + '-title').querySelectorAll('.ptab')].filter((t) => t.dataset.view !== drag.name)
    let index
    let at = tabs.length ? tabs[tabs.length - 1].getBoundingClientRect().right + 2 : strip.left
    for (const [i, t] of tabs.entries()) {
      const r = t.getBoundingClientRect()
      if (x < r.left + r.width / 2) {
        index = i
        at = r.left - 2
        break
      }
    }
    return { place: p, index, bar: { x: at, top: head.top + 5, height: head.height - 10 } }
  }
  const work = $('work').getBoundingClientRect()
  const main = $('main').getBoundingClientRect()
  if (x < work.left || x > work.right || y < work.top || y > work.bottom) return null
  const side = Math.min(320, (work.right - rail.right) * 0.25)
  if (x < rail.right + side) return { place: 'left', box: boxOf('left', main) }
  if (x > work.right - side) return { place: 'right', box: boxOf('right', main) }
  if (y > main.bottom - main.height * 0.35) return { place: 'bottom', box: boxOf('bottom', main) }
  return null
}

/** The area a place takes, or would take once open. */
function boxOf(p, main) {
  const el = $(PLACES[p].box)
  if (!el.hidden) return el.getBoundingClientRect()
  const css = getComputedStyle(document.documentElement)
  const size = (v, d) => parseFloat(css.getPropertyValue(v)) || d
  if (p === 'left') return { left: main.left, top: main.top, width: size('--side', 240), height: main.height }
  if (p === 'right') return { left: main.right - size('--right', 380), top: main.top, width: size('--right', 380), height: main.height }
  const hgt = size('--panel', 220)
  return { left: main.left, top: main.bottom - hgt, width: main.width, height: hgt }
}

function paintTarget(t) {
  drag.zone.hidden = !t?.box
  drag.line.hidden = !t?.rail && !t?.bar
  if (t?.box) Object.assign(drag.zone.style, { left: t.box.left + 'px', top: t.box.top + 'px', width: t.box.width + 'px', height: t.box.height + 'px' })
  else if (t?.rail) Object.assign(drag.line.style, { left: t.rail.left + 6 + 'px', top: t.line - 1 + 'px', width: t.rail.width - 12 + 'px', height: '2px' })
  else if (t?.bar) Object.assign(drag.line.style, { left: t.bar.x - 1 + 'px', top: t.bar.top + 'px', width: '2px', height: t.bar.height + 'px' })
}
