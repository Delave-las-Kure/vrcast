/**
 * Куда ведёт ссылка «развернуть» из карточки сервера.
 *
 * Сервер берётся из адреса, а не из «активного»: разворачивают обычно **не** тот сервер, с
 * которым сейчас работают, — активный раздаёт фильмы, а новый ещё пуст. Взять активный
 * значило бы развернуть не туда, и узналось бы это по тому, что раздача перестала работать.
 */

import { useSearchParams } from "react-router-dom";

import { DeployScreen } from "./DeployScreen";
import { useT } from "../../shared/i18n";

export function DeployPage() {
  const t = useT();
  const [params] = useSearchParams();
  const serverId = params.get("server");

  if (!serverId) return <p>{t.ui.servers.empty}</p>;
  return <DeployScreen serverId={serverId} />;
}
