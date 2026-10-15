import { useEffect, useSyncExternalStore } from "react";
import { useToast } from "../components/Toast";
import * as store from "./store";

export type { PendingMessage } from "./store";

/**
 * Messages of one conversation. `deviceId` is the conversation on the host;
 * a paired device passes null (it only has its own conversation).
 */
export function useConversation(deviceId: string | null) {
  const key = store.keyFor(deviceId);
  const state = useSyncExternalStore(
    (fn) => store.subscribe(key, fn),
    () => store.getConversation(key),
  );
  const toast = useToast();

  useEffect(() => {
    store.setErrorHandler((msg) => toast(msg, "error"));
    void store.ensureLoaded(key, deviceId);
    return () => store.setErrorHandler(null);
  }, [key, deviceId, toast]);

  return {
    ...state,
    loadOlder: () => store.loadOlder(key),
    sendText: (text: string) => store.sendText(key, text),
    sendFiles: (files: File[]) => store.sendFiles(key, files),
    retry: (id: string) => store.retry(key, id),
    discard: (id: string) => store.discard(key, id),
  };
}
