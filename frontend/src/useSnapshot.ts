import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Snapshot } from "./types";

export function useSnapshot() {
  const [snapshot, setSnapshot] = useState<Snapshot>();
  const [error, setError] = useState<string>();
  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        const next = await invoke<Snapshot>("snapshot");
        if (!cancelled) {
          setSnapshot((old) =>
            !old || next.revision >= old.revision ? next : old,
          );
          setError(undefined);
        }
      } catch (error) {
        if (!cancelled) setError(String(error));
      }
      if (!cancelled) timer = setTimeout(() => void poll(), 500);
    }
    void poll();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, []);
  return { snapshot, error };
}

export function useScreenshot(snapshot: Snapshot | undefined) {
  const [image, setImage] = useState<{
    session: number;
    url: string;
    at: number | null;
  }>();
  const [error, setError] = useState<{ session: number; message: string }>();
  const current = useRef<string | undefined>(undefined);
  const session = snapshot?.session?.id;
  const sequence = snapshot?.frame_sequence;
  useEffect(() => {
    let cancelled = false;
    if (session === undefined || sequence == null) {
      if (current.current) URL.revokeObjectURL(current.current);
      current.current = undefined;
      setImage(undefined);
      setError(undefined);
      return;
    }
    void invoke<ArrayBuffer>("screenshot", { sessionId: session, sequence })
      .then((bytes) => {
        if (cancelled) return;
        const next = URL.createObjectURL(
          new Blob([bytes], { type: "image/jpeg" }),
        );
        if (current.current) URL.revokeObjectURL(current.current);
        current.current = next;
        setImage({ session, url: next, at: snapshot?.captured_at_ms ?? null });
        setError(undefined);
      })
      .catch((error: unknown) => {
        if (!cancelled && !String(error).includes("superseded"))
          setError({ session, message: "Screenshot unavailable" });
      });
    return () => {
      cancelled = true;
    };
  }, [session, sequence]);
  useEffect(
    () => () => {
      if (current.current) URL.revokeObjectURL(current.current);
    },
    [],
  );
  return {
    image: image?.session === session ? image : undefined,
    error: error?.session === session ? error?.message : undefined,
  };
}
