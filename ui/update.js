import { notify } from './notify.js'
import { $, h, invoke, listen, app, state, settings, reg, toast, guard, modal } from './core.js'

const button = $('update')
let found = null

function offer(info) {
  found = info
  button.textContent = `↑ Update to ${info.version}`
  button.hidden = false
  app.fitBar?.()
}

export async function check(manual = false) {
  try {
    const info = await invoke('update_check')
    if (info) {
      offer(info)
      if (manual) showDialog()
      else notify({ id: 'update', title: `clight ${info.version} is available`, body: 'Open files are saved and clight restarts by itself.', actions: [{ label: 'Update and restart', primary: true, run: () => guard(install()) }, { label: 'Later' }] })
    } else if (manual) toast('clight is up to date')
  } catch (e) {
    if (manual) toast(`Couldn't check for updates: ${e}`)
  }
}

function showDialog() {
  if (!found) return
  let close
  close = modal(
    h(
      'div',
      { class: 'dialog' },
      h('h3', {}, `clight ${found.version} is available`),
      found.notes ? h('div', { class: 'notes' }, found.notes) : null,
      h('p', { class: 'muted' }, 'Open files are saved and clight restarts by itself.'),
      h(
        'div',
        { class: 'actions' },
        h('button', { onclick: () => close() }, 'Later'),
        h('button', { class: 'primary', onclick: () => (close(), guard(install())) }, 'Update and Restart'),
      ),
    ),
  )
}

async function install() {
  if (state.running || state.debugging) return toast('Stop the running program before updating')
  if (state.root && !(await app.saveAll())) return
  button.disabled = true
  button.textContent = 'Downloading…'
  try {
    await invoke('update_install')
  } catch (e) {
    button.disabled = false
    button.textContent = `↑ Update to ${found.version}`
    throw e
  }
}

listen('update-progress', (e) => {
  const { done, total } = e.payload
  if (total) button.textContent = `Downloading ${Math.floor((done / total) * 100)}%`
})

button.addEventListener('click', showDialog)

async function about() {
  const version = await invoke('app_version')
  let close
  close = modal(
    h(
      'div',
      { class: 'dialog' },
      h('h3', {}, 'clight'),
      h('p', { class: 'muted' }, `Version ${version}`),
      h('div', { class: 'actions' }, h('button', { onclick: () => (close(), guard(check(true))) }, 'Check for Updates'), h('button', { class: 'primary', onclick: () => close() }, 'Close')),
    ),
  )
}

reg('check-updates', 'Check for Updates…', null, () => check(true))
reg('about', 'About clight', null, about)

export function startup() {
  if (settings.updates) setTimeout(check, 6000)
}
