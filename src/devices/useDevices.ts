import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";
import { events } from "../lib/events";
import type { Device, Message } from "../lib/types";

/** Host-only: paired devices, kept fresh from server events. */
export function useDevices() {
  const [devices, setDevices] = useState<Device[] | null>(null);

  const reload = useCallback(async () => {
    try {
      setDevices(await api.get<Device[]>("/api/host/devices"));
    } catch {
      // keep the old list; the next event retries
    }
  }, []);

  useEffect(() => {
    void reload();
    const offs = [
      events.on("devices", () => void reload()),
      events.on("pairing_done", () => void reload()),
      events.on("resync", () => void reload()),
      events.on("cleared", () => void reload()),
      events.on("msg", (data) => {
        const m = data as Message;
        setDevices((prev) => prev?.map((d) => (d.id === m.device_id ? { ...d, last_message: m } : d)) ?? prev);
      }),
    ];
    return () => offs.forEach((off) => off());
  }, [reload]);

  return { devices, reload, setDevices };
}
