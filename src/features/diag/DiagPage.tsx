/**
 * Диагностика в разделе приложения.
 *
 * Сервер берётся из выбранного, а не из адреса: сюда приходят не из карточки сервера, а из
 * бокового меню — с жалобой «тормозит», а не с мыслью «посмотрю-ка вон тот сервер». Если ни
 * один не выбран, так и сказано: пустой экран человек прочитает как «всё хорошо».
 */

import { DiagScreen } from "./DiagScreen";
import { useActiveServer } from "../servers/store";
import { useT } from "../../shared/i18n";

export function DiagPage() {
  const t = useT();
  const server = useActiveServer();

  if (!server) return <p>{t.ui.viewers.noServer}</p>;
  return <DiagScreen serverId={server.id} />;
}
