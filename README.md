# devicectrl-server

Cental server for processing update requests from clients by sending update commands to devices and relaying state update notifications.

## Devices & Controllers

Devices types and their corresponding state and update structures are defined in [devicectrl-common](https://github.com/MatthewCash/devicectrl-common). Device controllers, found in `devices/controllers/` contain the core logic for dispatching state update commands to devices and processing state update notifications from devices.

## Server Backends

This server implements all of the following client protocols from the specifications described in [devicectrl-common](https://github.com/MatthewCash/devicectrl-common).

-   TCP
-   WebSocket
-   HTTP

## Automations

Automations are mini-programs run as tokio tasks that can perform actions based on events.

### Hooks

Hooks provide a tokio channel for automations to listen for events. An automation can iterate over a hook channel to run custom logic on device state update command dispatches or when a device reports a state update notification.

While the server backends are not technically automations, they use the same hooks and dispatch methods.

## Configuration

Example:

```json
{
    "servers": {
        "tcp": {
            "listen_on": "0.0.0.0:8894",
            "cert_path": "/run/credentials/devicectrl-server.service/server.crt",
            "key_path": "/run/credentials/devicectrl-server.service/server.key",
            "client_ca_path": "/run/credentials/devicectrl-server.service/ca.pem"
        },
        "http": {
            "listen_on": "0.0.0.0:8897",
            "cert_path": "/run/credentials/devicectrl-server.service/server.crt",
            "key_path": "/run/credentials/devicectrl-server.service/server.key",
            "client_ca_path": "/run/credentials/devicectrl-server.service/ca.pem"
        },
        "websocket": {
            "listen_on": "0.0.0.0:8896",
            "cert_path": "/run/credentials/devicectrl-server.service/server.crt",
            "key_path": "/run/credentials/devicectrl-server.service/server.key",
            "client_ca_path": "/run/credentials/devicectrl-server.service/ca.pem"
        }
    },
    "devices": {
        "lamp": {
            "device_type": "Switch",
            "controller": {
                "type": "Simple",
                "public_key_path": "/etc/devicectrl-server/lamp_public.der"
            }
        },
        "lights": {
            "device_type": "LedStrip",
            "controller": {
                "type": "Govee",
                "address": "192.168.1.150"
            }
        }
    },
    "controllers": {
        "Simple": {
            "listen_on": "0.0.0.0:8895",
            "server_private_key_path": "/etc/devicectrl-server/server_private.der"
        },
        "Tplink": {
            "response_timeout": { "secs": 1, "nanos": 0 }
        },
        "Govee": {
            "update_query_delay": { "secs": 0, "nanos": 50000000 }
        }
    },
    "automations": {
        "sunset_event": {
            "coords": [42.2, -119.1],
            "updates": [
                {
                    "device_id": "lamp",
                    "change_to": { "LedStrip": { "brightness": 50 } }
                }
            ]
        },
        "dependencies": [
            {
                "dependent_id": "lamp",
                "dependency_id": "light"
            }
        ]
    },
    "scenes": {
        "morning": [
            {
                "device_id": "lamp",
                "update": { "Brightness": { "brightness": 100 } }
            },
            {
                "device_id": "lamp2",
                "update": { "Brightness": { "brightness": 100 } }
            }
        ],
        "night": [
            {
                "device_id": "lamp",
                "update": { "Power": { "power": false } }
            },
            {
                "device_id": "lamp2",
                "update": { "Power": { "power": false } }
            }
        ]
    }
}
```

## Running

Simply execute:

`CONFIG_PATH=config.json cargo run`

Or use the provided systemd service:

```bash
cargo build --release

sudo install -m 755 ./target/release/devicectrl-server /usr/local/bin/
sudo install -m 644 devicectrl-server.service /etc/systemd/system/

sudo systemctl daemon-reload
sudo systemctl enable --now devicectrl-server
```
