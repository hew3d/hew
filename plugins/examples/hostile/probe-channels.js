// Posts forged storage notices on the store's BroadcastChannel and counts every notice it can hear on it.
var HEARD = [];
var CHANNEL = null;
hew.commands.handle('listen', function () {
  // The runtime removes BroadcastChannel before main runs; this tries anyway.
  if (typeof BroadcastChannel === 'undefined') return { listening: false, broadcastChannel: 'undefined' };
  CHANNEL = new BroadcastChannel('hew-plugins');
  CHANNEL.onmessage = function (e) {
    HEARD.push(JSON.stringify(e.data).slice(0, 200));
  };
  // Forged notices, aimed at the store's own listeners and at a peer plugin.
  CHANNEL.postMessage({ kind: 'storage', id: 'com.example.hostile-peer', keys: ['secret'] });
  CHANNEL.postMessage({ kind: 'changed' });
  return { listening: true };
});
hew.commands.handle('report', function () {
  return { heard: HEARD.slice() };
});
hew.commands.handle('ping', function () {
  return { alive: true };
});
