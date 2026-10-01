/**
 * T443 — small pure things about file names, shared by the screens.
 *
 * `slugOf` and `filmLabel` lived here too while the batch, quality and upload screens each
 * guessed a server name from a file. The «Video» screen (T673) leaves that to the core —
 * `video_add` makes the title and the short name — so only the plain name is left.
 */

/** The file's own name, without any of the path in front of it. */
export function basename(path: string): string {
  return path.split(/[\\/]/).pop() ?? "";
}
