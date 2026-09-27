// DOM images are logical runtime assets too. Verify/decode through PackClient
// before creating a short-lived presentation URL; no hash leaks to game code.
export class PackedImages {
  constructor(client) {
    this.client = client;
    this.active = 0;
    this.queue = [];
    this.urls = new Set();
    this.disposed = false;
  }
  bind(image, path) {
    if (this.disposed) return;
    this.queue.push({ image, path });
    this.pump();
  }
  pump() {
    while (!this.disposed && this.active < 2 && this.queue.length) {
      const { image, path } = this.queue.shift();
      this.active++;
      this.load(image, path)
        .catch(() => image.dispatchEvent(new Event("error")))
        .finally(() => {
          this.active--;
          this.pump();
        });
    }
  }
  async load(image, path) {
    const mime = {
      png: "image/png",
      jpg: "image/jpeg",
      jpeg: "image/jpeg",
      webp: "image/webp",
    }[path.split(".").pop()?.toLowerCase()];
    if (!mime) throw new Error("Unsupported packed DOM image type");
    const entry = await this.client.resolve(path);
    if (entry.bytes > 16 * 1048576)
      throw new Error("Packed DOM image exceeds its buffer limit");
    const bytes = await this.client.read(path);
    if (this.disposed) return;
    const url = URL.createObjectURL(new Blob([bytes], { type: mime }));
    this.urls.add(url);
    let released = false;
    const release = () => {
      if (released) return;
      released = true;
      URL.revokeObjectURL(url);
      this.urls.delete(url);
      image.removeEventListener("load", release);
      image.removeEventListener("error", release);
    };
    image.addEventListener("load", release, { once: true });
    image.addEventListener("error", release, { once: true });
    // Already downloaded and verified. Eager decoding ensures a detached or
    // re-paginated image cannot retain an unconsumed lazy blob URL indefinitely.
    image.loading = "eager";
    image.src = url;
  }
  dispose() {
    this.disposed = true;
    this.queue.length = 0;
    for (const url of this.urls) URL.revokeObjectURL(url);
    this.urls.clear();
  }
}
