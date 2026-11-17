// Shared by the hostile fixtures: reaches the realm port the way any plugin
// can (a `message` listener of its own sees the port the runtime is handed),
// so a fixture can send what the SDK never would. Not a fixture itself.

var RAW = (function () {
  var port = null;
  var rawSend = null;
  // The forwards the host has handed this plugin that it has not answered yet, oldest first.
  var open = [];
  // Forwards answered before this file's listener saw them (the runtime's listener runs first).
  var answered = {};
  var seq = 0;
  var waiting = {};
  var received = [];
  var nextOf = {};

  self.addEventListener('message', function (ev) {
    var d = ev.data;
    if (port !== null || !d || d.t !== 'port' || ev.ports.length !== 1) return;
    port = ev.ports[0];
    // Watches what the runtime sends, to know when a forward is answered.
    var send = port.postMessage.bind(port);
    rawSend = send;
    port.postMessage = function (message, transfer) {
      if (message && message.t === 'forward_reply') {
        answered[message.id] = true;
        open = open.filter(function (id) { return id !== message.id; });
      }
      return send(message, transfer);
    };
    port.addEventListener('message', function (m) {
      var data = m.data;
      if (!data || typeof data !== 'object') return;
      received.push(data.t);
      if (nextOf[data.t]) {
        nextOf[data.t](data);
        delete nextOf[data.t];
      }
      if (data.t === 'forward' && !answered[data.id]) open.push(data.id);
      if (data.t === 'bus' && typeof data.frame === 'string') {
        try {
          var reply = JSON.parse(data.frame);
          if (waiting[reply.id]) {
            waiting[reply.id](reply);
            delete waiting[reply.id];
          }
        } catch (e) {
          /* not a reply */
        }
      }
    });
  });

  return {
    /** True once the runtime's port has been seen. */
    get ready() {
      return port !== null;
    },
    /** The host's own number for the forward being served (the newest unanswered), or null. */
    get forward() {
      return open.length === 0 ? null : open[open.length - 1];
    },
    /** The message types the host has sent on the port. */
    get received() {
      return received.slice();
    },
    /** Posts anything straight on the port, with no SDK in between. */
    post: function (message, transfer) {
      // The runtime's own send, as it was before this file wrapped it.
      return rawSend(message, transfer || []);
    },
    /** Resolves with the next message of type `t` the host sends. */
    next: function (t) {
      return new Promise(function (resolve) {
        nextOf[t] = resolve;
      });
    },
    /** A JSON-RPC request sent at once, with no reply awaited. */
    busNow: function (method, params) {
      var frame = JSON.stringify({ jsonrpc: '2.0', id: 'now' + ++seq, method: method, params: params });
      rawSend({ t: 'bus', frame: frame });
    },
    /** One JSON-RPC request on the bus, named to the forward being served. */
    bus: function (method, params, forwardId) {
      var id = 'raw' + ++seq;
      // One task first: the host's forward message reaches this file's listener after the runtime's.
      return new Promise(function (resolve) {
        setTimeout(function () {
          var msg = { t: 'bus', frame: JSON.stringify({ jsonrpc: '2.0', id: id, method: method, params: params }) };
          var f = forwardId === undefined ? RAW.forward : forwardId;
          if (f !== null) msg.forward = f;
          waiting[id] = resolve;
          rawSend(msg);
        }, 0);
      });
    },
  };
})();

// A command's declared result is a closed object, so the fixtures declare one
// open slot, `r`, and every handler's answer goes in it. A function or
// nothing at all is passed on as it is, so those rows still send what they mean to.
(function () {
  var handle = hew.commands.handle.bind(hew.commands);
  var wrap = function (value) {
    return typeof value === 'function' || value === undefined ? value : { r: value };
  };
  hew.commands.handle = function (local, fn) {
    handle(local, function (params, ctx) {
      var value = fn(params, ctx);
      return value && typeof value.then === 'function' ? value.then(wrap) : wrap(value);
    });
  };
})();
