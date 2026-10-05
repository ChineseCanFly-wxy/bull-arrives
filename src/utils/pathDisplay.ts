/** Display only: pass the original path to filesystem operations and IPC. */
export function displayPath(path: string | null | undefined): string {
  if (!path) return '';
  if (/^\\\\\?\\UNC\\/i.test(path)) return '\\\\' + path.slice(8);
  if (/^\\\\\?\\[a-z]:\\/i.test(path)) return path.slice(4);
  return path;
}
