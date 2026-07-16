# @kloudlite/ws-pi

Use [pi](https://github.com/earendil-works/pi-coding-agent) against a remote `ws` workspace.

```sh
npm install -g @kloudlite/ws-pi
export WS_SERVER_URL=your-server.example
ws-pi
```

A hostname automatically uses HTTP and port `18765`. Without `WS_SERVER_URL`, `ws-pi` uses `localhost:18765`. Override it for one run:

```sh
ws-pi --server your-server.example
ws-pi --new
```

Start a compatible server with the [`ws` project](https://github.com/kloudlite/workspace-ide). Keep the server address private in your shell configuration rather than source code.
