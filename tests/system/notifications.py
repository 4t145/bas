"""Record notifications on a real session bus without a graphical desktop."""

import asyncio
import json
import os
from pathlib import Path

from dbus_next.aio import MessageBus
from dbus_next.service import ServiceInterface, method


class Notifications(ServiceInterface):
    def __init__(self):
        super().__init__("org.freedesktop.Notifications")
        self.next_id = 1
        self.log = Path(os.environ["XDG_RUNTIME_DIR"]) / "bas-notifications.jsonl"

    @method()
    def GetServerInformation(self) -> "ssss":
        return ["bas-system-test", "bas", "1.0", "1.2"]

    @method()
    def GetCapabilities(self) -> "as":
        return ["body"]

    @method()
    def Notify(
        self,
        app_name: "s",
        replaces_id: "u",
        app_icon: "s",
        summary: "s",
        body: "s",
        actions: "as",
        hints: "a{sv}",
        expire_timeout: "i",
    ) -> "u":
        notification_id = replaces_id or self.next_id
        self.next_id = max(self.next_id, notification_id + 1)
        with self.log.open("a") as stream:
            stream.write(json.dumps({
                "id": notification_id,
                "app": app_name,
                "title": summary,
                "body": body,
                "urgency": hints["urgency"].value,
            }) + "\n")
        return notification_id


async def main():
    bus = await MessageBus().connect()
    bus.export("/org/freedesktop/Notifications", Notifications())
    await bus.request_name("org.freedesktop.Notifications")
    await bus.wait_for_disconnect()


if __name__ == "__main__":
    asyncio.run(main())
