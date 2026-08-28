/**
 * T443 — the small pure things three screens each had a copy of.
 *
 * **Why they move rather than get copied a fourth time.** The batch screen names a directory
 * on the server for every film it puts in, and it has to be the same name the single-file
 * ladder screen would have chosen — otherwise the same film, put in twice by two routes,
 * lands in two places on the server and is served twice over. A fourth copy of the rule is a
 * fourth chance for that.
 */

/** The file's own name, without any of the path in front of it. */
export function basename(path: string): string {
  return path.split(/[\\/]/).pop() ?? "";
}

/**
 * The medium's own directory on the server, guessed from the file's name.
 *
 * A guess, and only until somebody is given somewhere to say otherwise. Kept to what is safe
 * in a path on either platform and in a URL: anything else becomes a hyphen.
 */
export function slugOf(path: string): string {
  return basename(path)
    .replace(/\.[^.]+$/, "")
    .replace(/[^A-Za-z0-9_-]+/g, "-")
    .toLowerCase();
}

/**
 * What to call a film on a task's row.
 *
 * The file's name without its extension, and untouched otherwise — this is for a person to
 * read, not for a filesystem. `slugOf` flattens the spaces and the Cyrillic out of a name,
 * which is right for a directory and wrong for a label: "Blue Eye Samurai S01E04" must not
 * arrive as "blue-eye-samurai-s01e04" on the one screen where somebody is looking for it.
 */
export function filmLabel(path: string): string {
  return basename(path).replace(/\.[^.]+$/, "");
}
