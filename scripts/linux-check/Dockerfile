# Образ для проверки сборки под Linux с машины разработчика.
#
# Существует ради одного класса ошибок: код под `#[cfg(unix)]` на Windows
# не компилируется вовсе, и любая ошибка в нём невидима до прогона в непрерывной
# интеграции. Десять минут ожидания ради лишнего импорта — плохой обмен.
#
# Состав повторяет то, что ставит CI (.github/workflows/build.yml): расхождение
# между этим образом и CI сделало бы проверку успокаивающей, но бесполезной.
#
# Версия НЕ закреплена намеренно. CI берёт `dtolnay/rust-toolchain@stable`, то есть
# всегда последнюю устойчивую; закреплённый здесь номер разошёлся бы с ней и начал
# давать замечания, которых в CI нет, — проверено на 1.90 против 1.98, где старый
# clippy ругался на то, что новый уже не считает ошибкой. Ложная тревога в проверке
# хуже её отсутствия: на неё перестают смотреть.

FROM rust:bookworm

RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      libwebkit2gtk-4.1-dev \
      libappindicator3-dev \
      librsvg2-dev \
      patchelf \
      pkg-config \
 && rm -rf /var/lib/apt/lists/*

RUN rustup component add rustfmt clippy

# Каталог сборки — внутри образа, а не в примонтированном исходнике: иначе
# сборки под Linux и под Windows затирали бы артефакты друг друга.
ENV CARGO_TARGET_DIR=/build/target
WORKDIR /app/src-tauri
