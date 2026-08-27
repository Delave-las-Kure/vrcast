/**
 * A row for a served file and its particulars (FR-012).
 *
 * What is unknown shows as a dash rather than being skipped or replaced with a zero.
 * The difference matters: "—" means "we could not read it", while "0" would read as
 * "the file does not have any".
 *
 * A file whose header is not at the start is called out separately: a viewer will only
 * start watching it after downloading the tail, and knowing that is more use than the
 * resolution and the bitrate put together.
 */

import type { FileView } from "../../shared/contract";
import { useLang, useT } from "../../shared/i18n";
import {
  formatBitrate,
  formatBytes,
  formatDuration,
  formatResolution,
} from "../../shared/i18n/format";
import { CopyLink } from "./CopyLink";

export function FileRow({ file, onDelete }: { file: FileView; onDelete?: (path: string) => void }) {
  const t = useT();
  const { lang } = useLang();
  const l = t.ui.library;

  return (
    <li className={`file ${file.exists_on_server ? "" : "file--missing"}`}>
      <div className="file__head">
        <span className="file__name">{file.path}</span>
        <span className="file__size">{formatBytes(file.size_bytes, lang)}</span>
      </div>

      <div className="file__meta">
        <span title={l.resolution}>{formatResolution(file.width, file.height)}</span>
        <span title={l.duration}>{formatDuration(file.duration_s)}</span>
        <span title={l.bitrate}>{formatBitrate(file.bitrate_bps, lang)}</span>
        {file.video_codec && <span title={l.video}>{file.video_codec}</span>}
        {file.audio_codec && <span title={l.audio}>{file.audio_codec}</span>}
      </div>

      {file.faststart_ok === false && <p className="file__warning">{l.faststartWarning}</p>}
      {!file.exists_on_server && <p className="file__warning">{l.missingWarning}</p>}

      <div className="file__actions">
        <CopyLink file={file} />
        {onDelete && (
          <button className="button--danger" onClick={() => onDelete(file.path)}>
            {l.deleteFile}
          </button>
        )}
      </div>
    </li>
  );
}
