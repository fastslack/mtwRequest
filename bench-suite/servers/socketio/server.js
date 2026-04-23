// Minimal Socket.IO server for the bench-suite.
//
// Events:
//   client → server: "join"     ({ channel: string })         → socket.join(channel)
//   client → server: "publish"  ({ channel, data })           → broadcast to room
//   server → client: "broadcast"({ channel, data })           → delivered to room
//
// GET /healthz returns 200 OK so docker-compose healthcheck can probe it.

const http = require('http');
const { Server } = require('socket.io');

const server = http.createServer((req, res) => {
  if (req.url === '/healthz') {
    res.writeHead(200, { 'Content-Type': 'text/plain' });
    res.end('ok');
    return;
  }
});

const io = new Server(server, {
  maxHttpBufferSize: 10e6,
  perMessageDeflate: false,
  pingInterval: 30_000,
  pingTimeout: 60_000,
});

io.on('connection', (socket) => {
  socket.on('join', (arg) => {
    const channel = (typeof arg === 'string') ? arg
                  : (arg && typeof arg.channel === 'string') ? arg.channel
                  : null;
    if (channel) socket.join(channel);
  });

  socket.on('publish', (msg) => {
    if (!msg || typeof msg.channel !== 'string') return;
    io.to(msg.channel).emit('broadcast', { channel: msg.channel, data: msg.data });
  });
});

const port = Number(process.env.PORT) || 3000;
server.listen(port, '0.0.0.0', () => {
  console.log(`bench-socketio listening on 0.0.0.0:${port}`);
});
