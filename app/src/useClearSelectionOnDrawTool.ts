import { useEffect } from 'react'

/**
 * Choosing a Draw tool at top level clears the selection: the user is about
 * to create geometry, so a still-selected object/group/component would pin
 * the contextual dock (and Object Info) to stale context. In-context
 * (edit-mode) selections are the drawing target and are kept.
 *
 * Keyed on the tool and on WHETHER a context is open, never on the context
 * path's identity: the path is re-sliced on every document change, and a
 * draw that forms a solid selects the new Object right after that change,
 * so an identity-keyed effect would clear that selection a render later.
 */
export function useClearSelectionOnDrawTool(
  activeTool: string,
  drawTools: ReadonlySet<string>,
  inContext: boolean,
  clear: () => void,
): void {
  useEffect(() => {
    if (!drawTools.has(activeTool)) return
    if (inContext) return
    clear()
    // `clear` and `drawTools` are stable by construction at the call site.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeTool, inContext])
}
