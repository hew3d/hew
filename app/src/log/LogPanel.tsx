/**
 * LogPanel — persistent collapsible log docked at the bottom of the app.
 *
 * Shows entries newest-first. Color-codes by severity. Provides a Clear button
 * and a collapse toggle. Does not use any CSS framework.
 */

import { useEffect, useRef, useState } from 'react'
import * as LogStore from './LogStore'
import type { LogEntry } from './LogStore'

/** Format a Date as HH:MM:SS */
function fmtTime(d: Date): string {
  const hh = String(d.getHours()).padStart(2, '0')
  const mm = String(d.getMinutes()).padStart(2, '0')
  const ss = String(d.getSeconds()).padStart(2, '0')
  return `${hh}:${mm}:${ss}`
}

const LEVEL_COLORS: Record<LogStore.LogLevel, { fg: string; label: string }> = {
  info: { fg: '#88ccff', label: 'INFO' },
  warn: { fg: '#ffcc44', label: 'WARN' },
  error: { fg: '#ff6655', label: 'ERR ' },
}

interface EntryRowProps {
  entry: LogEntry
}

function EntryRow({ entry }: EntryRowProps) {
  const { fg, label } = LEVEL_COLORS[entry.level]
  return (
    <div
      style={{
        display: 'flex',
        gap: '8px',
        padding: '2px 4px',
        fontFamily: 'monospace',
        fontSize: '11px',
        lineHeight: '1.4',
        borderBottom: '1px solid #2a2a2a',
        alignItems: 'baseline',
      }}
    >
      <span style={{ color: '#888', flexShrink: 0 }}>{fmtTime(entry.timestamp)}</span>
      <span style={{ color: fg, fontWeight: 'bold', flexShrink: 0, width: '30px' }}>{label}</span>
      <span style={{ color: '#aaa', flexShrink: 0 }}>[{entry.source}]</span>
      <span style={{ color: '#ddd', wordBreak: 'break-word', flex: 1 }}>{entry.message}</span>
    </div>
  )
}

interface Props {
  /** Height of the expanded panel body in pixels */
  panelHeight?: number
}

export function LogPanel({ panelHeight = 160 }: Props) {
  const [entries, setEntries] = useState<readonly LogEntry[]>(LogStore.getEntries())
  const [collapsed, setCollapsed] = useState(false)
  const scrollRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const unsub = LogStore.subscribe((e) => {
      setEntries([...e])
      // Auto-scroll to bottom on new entries (newest is last in the list;
      // we render reversed, so we scroll to scrollTop=0)
      if (scrollRef.current !== null) {
        scrollRef.current.scrollTop = 0
      }
    })
    return unsub
  }, [])

  const errorCount = entries.filter((e) => e.level === 'error').length
  const warnCount = entries.filter((e) => e.level === 'warn').length

  // Build badge summary for the header
  let badgeText = `${entries.length} entries`
  if (errorCount > 0 || warnCount > 0) {
    const parts: string[] = []
    if (errorCount > 0) parts.push(`${errorCount} err`)
    if (warnCount > 0) parts.push(`${warnCount} warn`)
    badgeText = parts.join(' · ')
  }

  return (
    <div
      style={{
        borderTop: '1px solid #444',
        background: '#1a1a1a',
        userSelect: 'none',
      }}
    >
      {/* Header bar */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          padding: '3px 8px',
          background: '#262626',
          cursor: 'pointer',
          gap: '8px',
        }}
        onClick={() => setCollapsed((c) => !c)}
      >
        <span style={{ color: '#aaa', fontFamily: 'monospace', fontSize: '11px' }}>
          {collapsed ? '▶' : '▼'} Log
        </span>
        <span
          style={{
            fontFamily: 'monospace',
            fontSize: '11px',
            color: errorCount > 0 ? '#ff6655' : warnCount > 0 ? '#ffcc44' : '#777',
          }}
        >
          {badgeText}
        </span>
        <div style={{ flex: 1 }} />
        <button
          onClick={(e) => {
            e.stopPropagation()
            LogStore.clear()
          }}
          style={{
            padding: '1px 8px',
            fontSize: '11px',
            fontFamily: 'monospace',
            background: '#383838',
            color: '#aaa',
            border: '1px solid #555',
            borderRadius: '2px',
            cursor: 'pointer',
          }}
        >
          Clear
        </button>
      </div>

      {/* Body */}
      {!collapsed && (
        <div
          ref={scrollRef}
          style={{
            height: `${panelHeight}px`,
            overflowY: 'auto',
            display: 'flex',
            flexDirection: 'column-reverse', // newest at top via CSS reverse
          }}
        >
          {/* Render in forward order; flexDirection: column-reverse flips them */}
          {[...entries].reverse().map((entry) => (
            <EntryRow key={entry.id} entry={entry} />
          ))}
          {entries.length === 0 && (
            <div
              style={{
                padding: '8px',
                color: '#555',
                fontFamily: 'monospace',
                fontSize: '11px',
                textAlign: 'center',
              }}
            >
              No log entries
            </div>
          )}
        </div>
      )}
    </div>
  )
}
