/**
 * The type every catalogue has to satisfy.
 *
 * The Russian one is the source of the shape: it was written first and the wordings
 * were carried over from the core word for word. Any other language must fill exactly
 * the same set of keys, and [`Widen`] frees it from having to repeat the Russian text
 * to satisfy a literal type.
 */

import type { ru } from "./ru";
import type { Widen } from "./types";

export type Catalogue = Widen<typeof ru>;
export type { Lang } from "./types";
