/*
 * Overlay client. Connects to the SSE stream and updates in place.
 *
 * OBS keeps a browser source alive across scene changes but will drop the
 * connection when the source is hidden or the app restarts, so reconnect
 * forever with a small backoff. An overlay that gives up after a few tries is
 * an overlay that is blank for the rest of the stream.
 */
;(function () {
  var el = function (id) {
    return document.getElementById(id)
  }
  var card = el('card')
  var art = el('art')
  var fill = el('fill')
  var lastKey = ''

  function fmt(s) {
    s = Math.max(0, Math.floor(s || 0))
    return Math.floor(s / 60) + ':' + String(s % 60).padStart(2, '0')
  }

  function render(d) {
    if (!d || !d.title) {
      card.hidden = true
      return
    }
    card.hidden = false
    card.classList.toggle('paused', !d.isPlaying)

    // Re-trigger the entry animation only on an actual track change.
    var key = d.title + '|' + d.artist
    if (key !== lastKey) {
      lastKey = key
      card.style.animation = 'none'
      void card.offsetWidth
      card.style.animation = ''
    }

    el('title').textContent = d.title
    el('artist').textContent = d.artist || ''
    el('album').textContent = [d.album, d.queueLength ? d.queueLength + ' queued' : '']
      .filter(Boolean)
      .join(' · ')

    if (d.requestedBy) {
      el('reqname').textContent = d.requestedBy
      el('req').hidden = false
    } else {
      el('req').hidden = true
    }

    if (d.artUrl) {
      if (art.getAttribute('src') !== d.artUrl) art.setAttribute('src', d.artUrl)
      art.hidden = false
    } else {
      art.hidden = true
    }

    var pct = d.durationSec ? Math.min(100, (d.positionSec / d.durationSec) * 100) : 0
    fill.style.width = pct + '%'
    el('pos').textContent = fmt(d.positionSec)
    el('dur').textContent = fmt(d.durationSec)
  }

  var retry = 0
  function connect() {
    var es = new EventSource('/events')
    es.onopen = function () {
      retry = 0
    }
    es.onmessage = function (e) {
      try {
        render(JSON.parse(e.data))
      } catch (_) {
        /* ignore a malformed frame rather than tearing down the stream */
      }
    }
    es.onerror = function () {
      es.close()
      retry = Math.min(retry + 1, 5)
      setTimeout(connect, 500 * Math.pow(2, retry - 1))
    }
  }
  connect()
})()
