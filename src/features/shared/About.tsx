/**
 * T107 — "About": where to get the source of the version in hand.
 *
 * Not decoration and not a formality. The application is distributed as a built file
 * under GPL-3.0-or-later, and the GPL requires that whoever received the build can get
 * its source too. The link must point at the **same** version they hold — which is why
 * the version number here comes from the core rather than from the markup: were it to
 * drift from the real one, the obligation would turn into a pretence.
 *
 * The list of third-party works in the package is generated at build time into
 * THIRD-PARTY.md (see `scripts/gen-third-party.mjs`). It is pointed at here, not
 * repeated: twenty thousand lines on screen help nobody, and a copy that gets
 * forgotten is worse than none.
 */

import { useEffect, useState } from "react";
import { ipc } from "../../shared/ipc";
import { useLang, useT } from "../../shared/i18n";
import { fill } from "../../shared/i18n/render";

/** Where the source lives. A GPL obligation, not a link "for the curious". */
const SOURCE_URL = "https://github.com/Delave-las-Kure/vrcast";

export function About() {
  const [version, setVersion] = useState<string | null>(null);
  const [schema, setSchema] = useState<number | null>(null);
  const t = useT();
  const { lang } = useLang();
  const a = t.ui.about;

  useEffect(() => {
    ipc
      .appVersions()
      .then((v) => {
        setVersion(v.app);
        setSchema(v.schema);
      })
      .catch(() => {
        setVersion(null);
        setSchema(null);
      });
  }, []);

  return (
    <div className="panel">
      <h1>{a.title}</h1>

      <p>
        <strong>VRCast Studio</strong>
        {version ? ` ${version}` : ""} — {a.tagline}
      </p>

      <h2>{a.licenceHeading}</h2>
      <p>
        {a.licenceBody1a} <strong>{a.licenceName}</strong>
        {a.licenceBody1b}
      </p>
      <p>
        {a.sourceLead} <em>{a.sourceThisVersion}</em>
        {version ? (
          <>
            {" "}
            ({a.sourceTag} <code>v{version}</code>)
          </>
        ) : null}{" "}
        {a.sourceAvailableAt}
      </p>
      <p>
        <a href={SOURCE_URL} target="_blank" rel="noreferrer">
          {SOURCE_URL}
        </a>
      </p>
      <p className="muted">{a.sourceMissing}</p>

      <h2>{a.thirdPartyHeading}</h2>
      <p>
        {a.thirdPartyBody} <code>THIRD-PARTY.md</code> {a.thirdPartyBodyTail}
      </p>
      <p>
        <a
          href={`${SOURCE_URL}/blob/main/THIRD-PARTY.md`}
          target="_blank"
          rel="noreferrer"
        >
          {a.thirdPartyLink}
        </a>
      </p>

      {schema !== null && (
        <p className="muted">{fill(a.schemaVersion, { schema }, t, lang)}</p>
      )}
    </div>
  );
}
