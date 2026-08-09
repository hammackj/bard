import {
  DndContext,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent
} from '@dnd-kit/core'
import { restrictToVerticalAxis, restrictToParentElement } from '@dnd-kit/modifiers'
import {
  SortableContext,
  useSortable,
  verticalListSortingStrategy
} from '@dnd-kit/sortable'
import { CSS } from '@dnd-kit/utilities'
import type { QueueItem } from '@shared/types'
import { useStore } from '../state/store'
import { fmtTime } from '../lib/format'

export function QueuePanel(): React.JSX.Element {
  const queue = useStore((s) => s.queue)

  // A small activation distance so a click to remove is not read as a drag.
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 4 } }))

  const onDragEnd = (e: DragEndEvent): void => {
    const { active, over } = e
    if (!over || active.id === over.id) return
    const from = queue.findIndex((q) => q.id === active.id)
    const to = queue.findIndex((q) => q.id === over.id)
    if (from === -1 || to === -1) return
    // Main owns the queue; it will broadcast the new order back.
    void window.bard.reorderQueue(from, to)
  }

  return (
    <section className="flex max-h-[38vh] flex-col border-t border-edge">
      <header className="flex items-center gap-3 px-4 pt-[13px] pb-[11px] text-s1 font-semibold tracking-[.13em] text-muted uppercase">
        <span>Queue</span>
        <span className="mono text-text">{queue.length}</span>
        <span className="ml-auto flex items-center gap-3 normal-case">
          <button
            className="tracking-normal hover:text-text disabled:opacity-40"
            onClick={() => void window.bard.shuffleQueue()}
            disabled={queue.length < 2}
            title="Shuffle the pending queue"
          >
            Shuffle
          </button>
          <button
            className="tracking-normal hover:text-text"
            onClick={() => void window.bard.fillQueue()}
            title="Add random tracks from the library"
          >
            Fill
          </button>
          <button
            className="tracking-normal hover:text-rose disabled:opacity-40"
            onClick={() => void window.bard.clearQueue()}
            disabled={queue.length === 0}
          >
            Clear
          </button>
        </span>
      </header>

      <div className="overflow-y-auto px-2 pb-2">
        {queue.length === 0 ? (
          <p className="px-2 py-3 text-s2 text-muted">
            Nothing queued. Pick a track, or hit Fill.
          </p>
        ) : (
          <DndContext
            sensors={sensors}
            collisionDetection={closestCenter}
            modifiers={[restrictToVerticalAxis, restrictToParentElement]}
            onDragEnd={onDragEnd}
          >
            <SortableContext items={queue.map((q) => q.id)} strategy={verticalListSortingStrategy}>
              {queue.map((item, i) => (
                <Row key={item.id} item={item} index={i} />
              ))}
            </SortableContext>
          </DndContext>
        )}
      </div>
    </section>
  )
}

function Row({ item, index }: { item: QueueItem; index: number }): React.JSX.Element {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: item.id
  })

  return (
    <div
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      className={`group grid grid-cols-[22px_1fr_auto_auto] items-baseline gap-3 rounded-sm px-2 py-[7px] hover:bg-panel ${
        isDragging ? 'relative z-10 bg-panel' : ''
      }`}
    >
      <span
        {...attributes}
        {...listeners}
        className="mono cursor-grab text-s1 text-muted active:cursor-grabbing"
        aria-label={`Reorder ${item.track.title}`}
      >
        {index + 1}
      </span>
      <span className="truncate text-s2">{item.track.title}</span>
      <span className="mono text-s1 text-muted">{fmtTime(item.track.durationSec)}</span>
      <span className="flex items-baseline gap-2 opacity-0 group-hover:opacity-100 focus-within:opacity-100">
        <button
          onClick={() => void window.bard.removeFromQueue(item.id)}
          className="text-s1 text-muted hover:text-rose"
          aria-label={`Remove ${item.track.title}`}
        >
          ✕
        </button>
      </span>
    </div>
  )
}
