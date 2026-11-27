/**
 * useAutoUpdateSetting — the state behind "Automatically download and
 * install updates", shared by the update window's checkbox, the macOS-style
 * Settings ▸ Advanced pane and the Windows mirror in FluentSettingsPage.tsx.
 *
 * The preference lives in the shell (update.json, beside server.json); it is
 * saved the moment it is toggled. `visible` is false on the web build, in a
 * build without an updater, and on a copy that cannot install by itself
 * (the shell reports `supported`), so no pane shows a switch that cannot work.
 */

import { useCallback, useEffect, useState } from 'react'
import { updateAutoGet, updateAutoSet, updaterAvailable } from '../updater/updateIpc'

export interface AutoUpdateSetting {
  visible: boolean
  enabled: boolean
  /** Saves at once; the shell's answer is what the box then shows. */
  setEnabled: (next: boolean) => void
}

export function useAutoUpdateSetting(): AutoUpdateSetting {
  const [state, setState] = useState<{ visible: boolean; enabled: boolean }>({ visible: false, enabled: false })

  useEffect(() => {
    let cancelled = false
    void (async () => {
      if (!(await updaterAvailable())) return
      try {
        const auto = await updateAutoGet()
        if (!cancelled) setState({ visible: auto.supported, enabled: auto.supported && auto.enabled })
      } catch {
        /* the command is absent or refused: leave the row hidden */
      }
    })()
    return () => {
      cancelled = true
    }
  }, [])

  const setEnabled = useCallback((next: boolean) => {
    setState((s) => ({ ...s, enabled: next }))
    void updateAutoSet(next)
      .then((auto) => setState({ visible: auto.supported, enabled: auto.enabled }))
      .catch(() => {
        // The save failed: show what is really stored.
        void updateAutoGet()
          .then((auto) => setState({ visible: auto.supported, enabled: auto.enabled }))
          .catch(() => {})
      })
  }, [])

  return { visible: state.visible, enabled: state.enabled, setEnabled }
}
