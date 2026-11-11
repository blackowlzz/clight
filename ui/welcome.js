import { $, h, icon, invoke, app, state, load, store, guard, toast, actions, keyLabel, basename } from './core.js'
import { customizeView } from './settings.js'

let page = 'projects'
let version = ''
let filter = ''

const recents = () => load('recent', [])
const shorten = (p) => (state.sys.home && p.startsWith(state.sys.home) ? '~' + p.slice(state.sys.home.length) : p)

function ago(t) {
  if (!t) return ''
  const m = Math.floor((Date.now() - t) / 60000)
  if (m < 1) return 'just now'
  if (m < 60) return `${m} min ago`
  const h = Math.floor(m / 60)
  if (h < 24) return `${h} h ago`
  const d = Math.floor(h / 24)
  return d === 1 ? 'yesterday' : d < 30 ? `${d} days ago` : new Date(t).toLocaleDateString()
}

// The colors of the project icons: a lighter and a darker shade of each.
const TILE_COLORS = [
  ['#6c9dff', '#3d6fe8'],
  ['#f0a35e', '#e0762e'],
  ['#b987f5', '#8b58dc'],
  ['#5fcb8e', '#2fa865'],
  ['#f27c8f', '#db4b64'],
  ['#4fc6d0', '#2399a8'],
  ['#e8c35a', '#cba030'],
  ['#9aa6f8', '#6f74e6'],
]

/** The icon of a project, the way the JetBrains IDEs draw it: initials in white on a color. */
export function tile(name) {
  let hash = 0
  for (const c of name) hash = (hash * 31 + c.charCodeAt(0)) >>> 0
  const words = name.split(/[\s_.-]+/).filter(Boolean)
  const letters = [...(words.length > 1 ? words[0][0] + words[1][0] : name.slice(0, 2))]
  const text = words.length > 1 ? letters.join('').toUpperCase() : letters[0].toUpperCase() + (letters[1] ?? '').toLowerCase()
  const [light, dark] = TILE_COLORS[hash % TILE_COLORS.length]
  const t = h('span', { class: 'ptile' }, text)
  t.style.background = `linear-gradient(145deg, ${light}, ${dark})`
  return t
}

async function projectRows(host) {
  const times = load('recentTime', {})
  const q = filter.toLowerCase()
  const list = recents().filter((p) => !q || p.toLowerCase().includes(q))
  const exists = await Promise.all(list.map((p) => invoke('path_exists', { path: p })))
  host.replaceChildren(
    ...list.map((p, i) =>
      h(
        'div',
        { class: 'proj' + (exists[i] ? '' : ' gone'), title: p, onclick: () => (exists[i] ? guard(app.openProject(p)) : toast('This folder no longer exists')) },
        tile(basename(p)),
        h('div', { class: 'pinfo' }, h('div', { class: 'pname' }, basename(p)), h('div', { class: 'ppath' }, exists[i] ? shorten(p) : 'Folder not found')),
        h('span', { class: 'ptime' }, ago(times[p])),
        h(
          'span',
          {
            class: 'px',
            title: 'Remove from list',
            onclick: (e) => {
              e.stopPropagation()
              store('recent', recents().filter((x) => x !== p))
              render()
            },
          },
          '×',
        ),
      ),
    ),
  )
  if (!list.length && recents().length) host.append(h('div', { class: 'notice' }, 'No matching projects'))
}

function actionTile(ic, title, sub, onclick) {
  return h('button', { class: 'atile', onclick }, h('span', { class: 'aic' }, icon(ic)), h('span', { class: 'at' }, title), h('span', { class: 'as' }, sub))
}

function projectsPage() {
  const has = recents().length > 0
  if (!has) {
    return h(
      'div',
      { class: 'wel-empty' },
      h('h1', {}, 'Welcome to clight'),
      h('p', {}, 'Create a project, open a folder on this computer, or clone one from Git.'),
      h(
        'div',
        { class: 'atiles' },
        actionTile('plus', 'New Project', 'C or C++ with CMake', app.newProject),
        actionTile('folder', 'Open', 'A folder on this computer', app.pickFolder),
        actionTile('branch', 'Clone', 'A repository from Git', app.cloneRepo),
      ),
    )
  }
  const list = h('div', { class: 'plist' })
  const search = h('div', { class: 'psearch' }, icon('search'), h('input', {
    placeholder: 'Search projects',
    spellcheck: 'false',
    value: filter,
    oninput: (e) => {
      filter = e.target.value
      projectRows(list)
    },
  }))
  projectRows(list)
  setTimeout(() => search.querySelector('input').focus())
  return h(
    'div',
    { class: 'wel-page' },
    h('div', { class: 'wel-top' }, search, h('button', { onclick: app.newProject }, 'New Project'), h('button', { onclick: app.pickFolder }, 'Open'), h('button', { onclick: app.cloneRepo }, 'Clone')),
    list,
  )
}

function learnPage() {
  const groups = [
    ['Find your way', ['actions', 'quick-open', 'recent', 'symbols', 'find-project', 'goto-def', 'usages']],
    ['Write code', ['code-actions', 'rename', 'format', 'comment', 'duplicate', 'expand']],
    ['Build and debug', ['build', 'run', 'debug', 'toggle-bp', 'dbg-over', 'dbg-into', 'dbg-resume']],
  ]
  return h(
    'div',
    { class: 'wel-page learn' },
    h('h2', {}, 'Shortcuts worth knowing'),
    h(
      'div',
      { class: 'lgrid' },
      ...groups.map(([title, ids]) =>
        h(
          'div',
          {},
          h('div', { class: 'eyebrow' }, title),
          ...ids.map((id) => actions.get(id)).filter(Boolean).map((a) => h('div', { class: 'lrow' }, h('span', {}, a.title), h('kbd', {}, keyLabel(a.key)))),
        ),
      ),
    ),
  )
}

export async function renderHome() {
  version ||= await invoke('app_version')
  render()
}

function render() {
  const nav = (id, label) =>
    h(
      'button',
      {
        class: page === id ? 'on' : '',
        onclick: () => {
          page = id
          if (id === 'learn') app.loadDebugActions().then(render)
          else render()
        },
      },
      label,
    )
  const content = page === 'projects' ? projectsPage() : page === 'customize' ? h('div', { class: 'wel-page' }, customizeView()) : learnPage()
  $('home').replaceChildren(
    h(
      'aside',
      { class: 'wel-side' },
      h('div', { class: 'wel-brand' }, h('span', { class: 'logo' }, h('span', { class: 'mark' })), h('div', {}, h('div', { class: 'wn' }, 'clight'), h('div', { class: 'wv' }, version))),
      h('nav', { class: 'wel-nav' }, nav('projects', 'Projects'), nav('customize', 'Customize'), nav('learn', 'Learn')),
      h('span', { class: 'grow' }),
      h('button', { class: 'wel-link', onclick: () => actions.get('check-updates').run() }, 'Check for Updates'),
    ),
    h('main', { class: 'wel-main' }, content),
  )
}
