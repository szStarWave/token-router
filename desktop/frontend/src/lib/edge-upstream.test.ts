import assert from 'node:assert/strict'
import { beforeEach, describe, it } from 'node:test'
import { getEdgeStoreState } from '../stores/edgeStore'
import {
  buildDisplayItems,
  initEdgeUpstream,
  syncEdgeFromSetup,
  upsertManualEntry,
} from './edge-upstream'

const EDGE_USER_CONFIGURED_KEY = 'tr-edge-user-configured'
const EDGE_MANUAL_ENTRIES_KEY = 'tr-edge-manual-entries'

const storage = new Map<string, string>()

function installLocalStorageMock(): void {
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => {
        storage.set(key, value)
      },
      removeItem: (key: string) => {
        storage.delete(key)
      },
      clear: () => {
        storage.clear()
      },
      key: () => null,
      length: 0,
    },
  })
}

function resetEdgeStore(): void {
  getEdgeStoreState().setManualEntries([])
  getEdgeStoreState().setSelectedKey(null)
  getEdgeStoreState().setPendingSetupSelection(null)
  getEdgeStoreState().setCachedModels([])
  getEdgeStoreState().setHerdsmanConnected(false)
  getEdgeStoreState().setHerdsmanInstalled(false)
  getEdgeStoreState().setEdgeBootReconciled(false)
  getEdgeStoreState().setPendingEdgeReconcile(null)
}

describe('edge-upstream manual model dedupe', () => {
  beforeEach(() => {
    storage.clear()
    installLocalStorageMock()
    resetEdgeStore()
    localStorage.setItem(EDGE_USER_CONFIGURED_KEY, '1')
  })

  it('does not create a duplicate when syncEdgeFromSetup receives a normalized URL', () => {
    upsertManualEntry({
      id: 'manual-1',
      name: "f's'd'f's'da",
      base_url: 'dsfsd',
      model: 'sadfsdaf',
    })

    syncEdgeFromSetup({
      base_url: 'http://dsfsd',
      model: 'sadfsdaf',
    })

    const entries = getEdgeStoreState().manualEntries
    assert.equal(entries.length, 1)
    assert.equal(entries[0]?.name, "f's'd'f's'da")
    assert.equal(entries[0]?.base_url, 'http://dsfsd')
    assert.equal(entries[0]?.model, 'sadfsdaf')
  })

  it('buildDisplayItems shows one manual item for equivalent endpoints', () => {
    getEdgeStoreState().setManualEntries([
      {
        id: 'manual-1',
        name: "f's'd'f's'da",
        base_url: 'dsfsd',
        model: 'sadfsdaf',
      },
      {
        id: 'manual-2',
        name: 'sadfsdaf',
        base_url: 'http://dsfsd',
        model: 'sadfsdaf',
        fromSetupRestore: true,
      },
    ])

    const manualItems = buildDisplayItems().filter((item) => item.type === 'manual')
    assert.equal(manualItems.length, 1)
    assert.equal(manualItems[0]?.name, "f's'd'f's'da")
  })

  it('loadManualEntriesFromStorage migrates and dedupes legacy duplicate entries', async () => {
    localStorage.setItem(
      EDGE_MANUAL_ENTRIES_KEY,
      JSON.stringify([
        {
          id: 'manual-1',
          name: "f's'd'f's'da",
          base_url: 'dsfsd',
          model: 'sadfsdaf',
        },
        {
          id: 'manual-2',
          name: 'sadfsdaf',
          base_url: 'http://dsfsd',
          model: 'sadfsdaf',
          fromSetupRestore: true,
        },
      ]),
    )

    await initEdgeUpstream()

    const entries = getEdgeStoreState().manualEntries
    assert.equal(entries.length, 1)
    assert.equal(entries[0]?.name, "f's'd'f's'da")
    assert.equal(entries[0]?.base_url, 'http://dsfsd')

    const persisted = JSON.parse(localStorage.getItem(EDGE_MANUAL_ENTRIES_KEY) || '[]') as unknown[]
    assert.equal(persisted.length, 1)
  })
})
