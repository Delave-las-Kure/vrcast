/**
 * T056 — копирование зрительской ссылки (FR-016).
 *
 * Когда CDN задан, ссылок две, и выбор оставляется человеку: у вариантов разная
 * цена. Ссылка через сам сервер в России не блокируется; через CDN быстрее, но
 * зависит от посредника и какое-то время отдаёт из кеша даже заменённый файл.
 *
 * Подтверждение копирования обязательно и держится пару секунд: без него человек
 * не знает, сработало ли нажатие, и жмёт ещё раз.
 */

import { useEffect, useRef, useState } from "react";
import type { FileView } from "../../shared/contract";

export function CopyLink({ file }: { file: FileView }) {
  const [copied, setCopied] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Отметку о копировании снимаем по таймеру, а таймер обязаны погасить при
  // размонтировании: иначе он сработает на уже убранном со страницы узле.
  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );

  const copy = async (url: string, what: string) => {
    try {
      await navigator.clipboard.writeText(url);
      setCopied(what);
    } catch {
      // Буфер обмена может быть недоступен. Молчать нельзя: человек решит,
      // что ссылка скопирована, и вставит не то.
      setCopied("не удалось");
    }
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => setCopied(null), 2000);
  };

  if (!file.exists_on_server) {
    return (
      <span className="copy-link__dead" title="Файла нет на сервере">
        ссылка нерабочая
      </span>
    );
  }

  return (
    <span className="copy-link">
      <button onClick={() => void copy(file.origin_url, "с сервера")}>
        {file.cdn_url ? "Ссылка с сервера" : "Копировать ссылку"}
      </button>
      {file.cdn_url && (
        <button onClick={() => void copy(file.cdn_url!, "через CDN")}>
          Ссылка через CDN
        </button>
      )}
      {copied && (
        <span className="copy-link__done" role="status">
          {copied === "не удалось" ? "скопировать не вышло" : `скопирована ${copied}`}
        </span>
      )}
    </span>
  );
}
