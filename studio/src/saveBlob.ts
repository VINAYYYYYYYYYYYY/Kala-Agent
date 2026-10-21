// Save a downloaded blob under a filename.
//
// One helper for every download control (the export panel, the design card's
// digest chips) so the anchor lifecycle, the `noopener` relation and the
// object-URL revocation exist once.

/** Store `blob` under `filename` through one transient anchor. */
export function saveBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = filename;
  anchor.rel = 'noopener';
  document.body.appendChild(anchor);
  anchor.click();
  document.body.removeChild(anchor);
  URL.revokeObjectURL(url);
}
