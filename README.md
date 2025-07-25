# devicectrl-server

Cental server for processing update requests from clients by sending update commands to devices and relaying state update notifications.

## Devices & Controllers

Devices types and their corresponding state and update structures are defined in [devicectrl-common](https://github.com/MatthewCash/devicectrl-common). Device controllers, found in `devices/controllers/` contain the core logic for dispatching state update commands to devices and processing state update notifications from devices.

## Server Backends

### TCP

Uses mtls to provide authentication and confidentiality meaning that the client must also present a certificate trusted by the server.

The TCP server and clients must serialize messages with JSON, delineating messages with a newline (`\n`). The enums defining messages are defined in [devicectrl-common](https://github.com/MatthewCash/devicectrl-common).

### HTTP

Currently unimplemented

### WebSocket

Nearly identical to the TCP server but using websocket semantics, mtls is used to provide authentication and confidentiality.

The websocket server and clients must only send text messages, serialized with JSON. Currently the websocket server also sends and expects to receive the same message enums as the TCP server.

## Automations

Automations are mini-programs run as tokio tasks that can perform actions based on events.

### Hooks

Hooks provide a tokio channel for automations to listen for events. An automation can iterate over a hook channel to run custom logic on device state update command dispatches or when a device reports a state update notification.

While the server backends are not automations, they function similarly by using the same hooks and dispatch methods.
