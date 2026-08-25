/**
 * Строка файла раздачи с его параметрами (FR-012).
 *
 * Неизвестное показывается прочерком, а не пропускается и не подменяется нулём.
 * Разница существенная: «—» означает «мы не смогли прочитать», а «0» читалось бы
 * как «в файле этого нет».
 *
 * Отдельно отмечается файл, у которого заголовок лежит не в начале: зритель начнёт
 * смотреть его только после скачивания хвоста, и знать об этом полезнее, чем
 * разрешение и битрейт вместе взятые.
 */

import type { FileView } from "../../shared/contract";
import {
  formatBitrate,
  formatBytes,
  formatDuration,
  formatResolution,
} from "../../shared/format";
import { CopyLink } from "./CopyLink";

export function FileRow({
  file,
  onDelete,
}: {
  file: FileView;
  onDelete?: (path: string) => void;
}) {
  return (
    <li className={`file ${file.exists_on_server ? "" : "file--missing"}`}>
      <div className="file__head">
        <span className="file__name">{file.path}</span>
        <span className="file__size">{formatBytes(file.size_bytes)}</span>
      </div>

      <div className="file__meta">
        <span title="Разрешение">{formatResolution(file.width, file.height)}</span>
        <span title="Длительность">{formatDuration(file.duration_s)}</span>
        <span title="Средний битрейт">{formatBitrate(file.bitrate_bps)}</span>
        {file.video_codec && <span title="Видео">{file.video_codec}</span>}
        {file.audio_codec && <span title="Звук">{file.audio_codec}</span>}
      </div>

      {file.faststart_ok === false && (
        <p className="file__warning">
          Заголовок не в начале файла — зритель начнёт смотреть только после того, как
          скачает его целиком. Такой файл стоит подготовить заново.
        </p>
      )}
      {!file.exists_on_server && (
        <p className="file__warning">
          Файла нет на сервере: его удалили или переименовали мимо приложения. Ссылка
          на него не работает.
        </p>
      )}

      <div className="file__actions">
        <CopyLink file={file} />
        {onDelete && (
          <button className="button--danger" onClick={() => onDelete(file.path)}>
            Удалить файл
          </button>
        )}
      </div>
    </li>
  );
}
