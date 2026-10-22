import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../lib/api";
import { events } from "../lib/events";
import type { Device, Message } from "../lib/types";

type Counts = Record<string, number>;

const withUnread = (list: Device[], counts: Counts) => list.map((d) => ({ ...d, unread: counts[d.id] ?? 0 }));

/** Host-only: paired devices, kept fresh from server events. */
export function useDevices() {
  const [devices, setDevices] = useState<Device[] | null>(null);
  // The newest unread counts the server announced, and how many announcements so far.
  const counts = useRef<Counts | null>(null);
  const announced = useRef(0);

  const reload = useCallback(async () => {
    const before = announced.current;
    try {
      const list = await api.get<Device[]>("/api/host/devices");
      // Counts announced while the list was on its way may be newer than its own.
      setDevices(announced.current !== before && counts.current ? withUnread(list, counts.current) : list);
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
      // Sent after every change to what is unread, so it is always the latest.
      events.on("unread", (data) => {
        const next = (data as { counts?: Counts } | null)?.counts;
        if (!next) return;
        counts.current = next;
        announced.current += 1;
        setDevices((prev) => (prev ? withUnread(prev, next) : prev));
      }),
    ];
    return () => offs.forEach((off) => off());
  }, [reload]);

  return { devices, reload, setDevices };
}
