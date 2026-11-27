// Posts junk shapes straight on the realm port, bypassing the SDK: bad types, sizes, ids, ports and floods.
function bigObject(n) {
  var o = { t: 'pong', n: 0 };
  for (var i = 0; i < n; i++) o['k' + i] = i;
  return o;
}

var SHAPES = {
  'oversize-frame': function () {
    RAW.post({ t: 'bus', frame: 'x'.repeat(2 << 20) });
  },
  'oversize-console-arg': function () {
    RAW.post({ t: 'console', level: 'log', args: ['x'.repeat(5000)], at: 1 });
  },
  'forged-forward-reply': function () {
    RAW.post({ t: 'forward_reply', id: 987654, reply: '{"ok":{}}' });
  },
  'forged-reply-shape': function () {
    RAW.post({ t: 'forward_reply', id: RAW.forward, reply: '{"ok":1,"also":2}' });
  },
  'forged-progress': function () {
    RAW.post({ t: 'forward_progress', id: 987654, fraction: 0.5 });
  },
  'forged-bus-forward': function () {
    RAW.post({ t: 'bus', forward: 987654, frame: JSON.stringify({ jsonrpc: '2.0', id: 'f', method: 'hew.meta.capabilities', params: {} }) });
  },
  'reused-storage-id': function () {
    RAW.post({ t: 'storage', id: 7, op: 'get', key: 'a' });
    RAW.post({ t: 'storage', id: 7, op: 'get', key: 'b' });
  },
  'negative-storage-id': function () {
    RAW.post({ t: 'storage', id: -1, op: 'get', key: 'a' });
  },
  'forged-hello-ack': function () {
    RAW.post({ t: 'hello_ack' });
  },
  'unknown-type': function () {
    RAW.post({ t: 'eval', code: '1' });
  },
  'extra-field': function () {
    RAW.post({ t: 'pong', n: 0, extra: 1 });
  },
  'bad-console-level': function () {
    RAW.post({ t: 'console', level: 'fatal', args: [], at: 1 });
  },
  'bad-fraction': function () {
    RAW.post({ t: 'forward_progress', id: RAW.forward, fraction: 7 });
  },
  'bad-forward-type': function () {
    RAW.post({ t: 'bus', forward: { a: { a: { a: {} } } }, frame: 'x' });
  },
  'proto-key': function () {
    RAW.post(JSON.parse('{"t":"pong","n":0,"__proto__":{"t":"eval"}}'));
  },
  'many-keys': function () {
    RAW.post(bigObject(200000));
  },
  'string-message': function () {
    RAW.post('hello');
  },
  'number-message': function () {
    RAW.post(42);
  },
  'null-message': function () {
    RAW.post(null);
  },
  'array-message': function () {
    RAW.post([{ t: 'pong', n: 0 }]);
  },
  'buffer-message': function () {
    RAW.post(new ArrayBuffer(8));
  },
  'port-transfer': function () {
    var channel = new MessageChannel();
    RAW.post({ t: 'pong', n: 0 }, [channel.port1]);
  },
  'big-transferred-buffer': function () {
    var buffer = new ArrayBuffer(200 << 20);
    RAW.post({ t: 'bus', frame: 'x', buffer: buffer }, [buffer]);
  },
  'message-flood': function () {
    for (var i = 0; i < 3000; i++) RAW.post({ t: 'pong', n: 0 });
  },
  'byte-flood': function () {
    for (var i = 0; i < 20; i++) RAW.post({ t: 'bus', frame: 'x'.repeat(1 << 20) });
  },
};

hew.commands.handle('run', async function (params) {
  // One task first: the host's forward message reaches the raw listener after the runtime's.
  await new Promise(function (resolve) {
    setTimeout(resolve, 0);
  });
  var shape = (params && params.shape) || (HOSTILE && HOSTILE.shape);
  if (shape === 'files') {
    var reply = RAW.next('files');
    RAW.post({
      t: 'files',
      id: 1,
      paths: [
        'dist/background.js',
        'hew-plugin.json',
        '../com.example.hostile-peer/hew-plugin.json',
        'dist/../../com.example.hostile-peer/hew-plugin.json',
        '/etc/passwd',
        'C:\\Windows\\win.ini',
        'dist//background.js',
        './dist/background.js',
        'nothing.txt',
      ],
    });
    var got = await reply;
    return {
      entries: got.entries.map(function (e) {
        return { path: e.path, bytes: e.bytes.byteLength };
      }),
      missing: got.missing,
    };
  }
  var send = SHAPES[shape];
  if (!send) throw new HewRefusal('unknown_shape', 'No such shape: ' + shape);
  send();
  // Wait for the host to react; a violation ends this Worker before the reply goes out.
  await new Promise(function (resolve) {
    setTimeout(resolve, 3000);
  });
  return { survived: true };
});
hew.commands.handle('ping', function () {
  return { alive: true };
});
