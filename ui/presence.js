// What this window tells Discord: the file being edited and the project it belongs to.
import { invoke, state, settings, activeTab, basename } from './core.js'

const appStart = Math.floor(Date.now() / 1000)
let since = appStart
let project = null
let sent = ''
let timer = 0

function push() {
  let activity = null
  if (settings.discord) {
    if (state.root !== project) {
      project = state.root
      since = project ? Math.floor(Date.now() / 1000) : appStart
    }
    const name = state.root ? basename(state.root) : ''
    const tab = activeTab()
    if (tab) activity = { details: `Editing ${tab.virtual ? tab.name || 'a file' : basename(tab.path)}`, state: name && `in ${name}`, start: since }
    else if (name) activity = { details: `Browsing ${name}`, state: '', start: since }
    else activity = { details: 'Idle', state: '', start: since }
  }
  const key = JSON.stringify(activity)
  if (key === sent) return
  sent = key
  invoke('discord_presence', { activity }).catch(() => {})
}

/** Call it whenever the open file or the project may have changed. Quick changes are sent together. */
export function sync() {
  clearTimeout(timer)
  timer = setTimeout(push, 700)
}

sync()
