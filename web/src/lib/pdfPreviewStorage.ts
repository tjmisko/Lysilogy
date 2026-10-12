const DATABASE = "lysilogy-pdf-previews-v1";
const STORE = "previews";
const MAX_ENTRIES = 1000;
// Paper IDs are path-derived; expire images so a replaced PDF is refreshed.
const MAX_AGE = 24 * 60 * 60 * 1000;
type StoredPreview = { url: string; image: string; savedAt: number };
let database: Promise<IDBDatabase> | undefined;

function openDatabase(): Promise<IDBDatabase> {
  database ??= new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open(DATABASE, 1);
    let blocked = false;
    request.onupgradeneeded = () => {
      request.result.createObjectStore(STORE, { keyPath: "url" }).createIndex("savedAt", "savedAt");
    };
    request.onsuccess = () => {
      if (blocked) { request.result.close(); return; }
      request.result.onversionchange = () => { request.result.close(); database = undefined; };
      resolve(request.result);
    };
    request.onerror = () => reject(request.error ?? new Error("Preview storage unavailable"));
    request.onblocked = () => { blocked = true; reject(new Error("Preview storage blocked")); };
  });
  return database;
}

export const pdfPreviewStorage = {
  async read(url: string): Promise<string | undefined> {
    const db = await openDatabase();
    return new Promise((resolve, reject) => {
      const transaction = db.transaction(STORE, "readonly");
      const request = transaction.objectStore(STORE).get(url);
      request.onsuccess = () => {
        const entry = request.result as StoredPreview | undefined;
        resolve(entry !== undefined && Date.now() - entry.savedAt < MAX_AGE ? entry.image : undefined);
      };
      transaction.onabort = () => reject(transaction.error ?? new Error("Preview read failed"));
    });
  },
  async write(url: string, image: string): Promise<void> {
    const db = await openDatabase();
    return new Promise((resolve, reject) => {
      // IndexedDB serializes these transactions, keeping eviction and writes atomic.
      const transaction = db.transaction(STORE, "readwrite");
      const store = transaction.objectStore(STORE);
      store.put({ url, image, savedAt: Date.now() } satisfies StoredPreview);
      const count = store.count();
      count.onsuccess = () => {
        let excess = count.result - MAX_ENTRIES;
        if (excess <= 0) return;
        const cursor = store.index("savedAt").openCursor();
        cursor.onsuccess = () => {
          if (cursor.result === null || excess <= 0) return;
          cursor.result.delete();
          excess -= 1;
          cursor.result.continue();
        };
      };
      transaction.oncomplete = () => resolve();
      transaction.onabort = () => reject(transaction.error ?? new Error("Preview write failed"));
    });
  },
};
