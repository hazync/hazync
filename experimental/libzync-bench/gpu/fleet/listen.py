import os, socket
open("/workspace/listen.pid", "w").write(str(os.getpid()))
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind(("0.0.0.0", 9110)); s.listen(64)
while True:
    c, _ = s.accept(); c.close()
