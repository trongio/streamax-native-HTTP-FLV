#!/usr/bin/env python3
"""
Streamax HTTP-FLV demo player.
Paste the live.flv URL, hit Play. mpv opens in its own window.
Requires: mpv installed (sudo apt install mpv).
"""

import shutil
import signal
import subprocess
import tkinter as tk
from tkinter import messagebox, ttk

MPV_FLAGS = [
    "--profile=low-latency",
    "--cache=yes",
    "--demuxer-max-bytes=10M",
    "--network-timeout=10",
    "--hwdec=auto",
    "--no-terminal",
    "--force-window=yes",
    "--keep-open=no",
    "--title=Streamax Demo — ${filename}",
]


class App:
    def __init__(self, root: tk.Tk) -> None:
        self.root = root
        self.proc: subprocess.Popen | None = None

        root.title("Streamax Native Player Demo")
        root.geometry("720x180")
        root.minsize(560, 160)

        frm = ttk.Frame(root, padding=14)
        frm.pack(fill="both", expand=True)

        ttk.Label(frm, text="Live FLV URL").grid(row=0, column=0, sticky="w")
        self.url_var = tk.StringVar(
            value="https://YOUR-CAMERA-HOST.example.com:22060/live.flv?devid=&chl=1&st=1&audio=1&hash=x"
        )
        self.entry = ttk.Entry(frm, textvariable=self.url_var)
        self.entry.grid(row=1, column=0, columnspan=3, sticky="ew", pady=(2, 10))

        self.play_btn = ttk.Button(frm, text="▶  Play", command=self.play)
        self.play_btn.grid(row=2, column=0, sticky="w")

        self.stop_btn = ttk.Button(frm, text="■  Stop", command=self.stop, state="disabled")
        self.stop_btn.grid(row=2, column=1, sticky="w", padx=(8, 0))

        self.status = ttk.Label(frm, text="Idle.", foreground="#555")
        self.status.grid(row=2, column=2, sticky="e", padx=(8, 0))

        frm.columnconfigure(2, weight=1)

        root.protocol("WM_DELETE_WINDOW", self.on_close)
        root.bind("<Return>", lambda _e: self.play())
        root.after(500, self.check_alive)

    def play(self) -> None:
        if self.proc and self.proc.poll() is None:
            messagebox.showinfo("Already playing", "Stop the current stream first.")
            return

        url = self.url_var.get().strip()
        if not url.startswith(("http://", "https://")):
            messagebox.showerror("Bad URL", "Paste an http(s) FLV URL.")
            return

        if not shutil.which("mpv"):
            messagebox.showerror(
                "mpv not found",
                "Install it first:\n\n  sudo apt install mpv",
            )
            return

        try:
            self.proc = subprocess.Popen(
                ["mpv", *MPV_FLAGS, url],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
        except OSError as exc:
            messagebox.showerror("Failed to launch mpv", str(exc))
            return

        self.status.config(text=f"Playing (pid {self.proc.pid}).", foreground="#0a7d2c")
        self.play_btn.state(["disabled"])
        self.stop_btn.state(["!disabled"])

    def stop(self) -> None:
        if self.proc and self.proc.poll() is None:
            try:
                self.proc.send_signal(signal.SIGTERM)
                self.proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                self.proc.kill()
        self.proc = None
        self.status.config(text="Stopped.", foreground="#555")
        self.play_btn.state(["!disabled"])
        self.stop_btn.state(["disabled"])

    def check_alive(self) -> None:
        if self.proc and self.proc.poll() is not None:
            self.proc = None
            self.status.config(text="Player exited.", foreground="#a13")
            self.play_btn.state(["!disabled"])
            self.stop_btn.state(["disabled"])
        self.root.after(500, self.check_alive)

    def on_close(self) -> None:
        self.stop()
        self.root.destroy()


if __name__ == "__main__":
    root = tk.Tk()
    App(root)
    root.mainloop()
