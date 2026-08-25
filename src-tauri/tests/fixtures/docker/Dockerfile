# Одноразовый сервер для интеграционных тестов (T026).
#
# Ubuntu 24.04 — та же система, что на боевом сервере и на тестовом VPS. Это не
# формальность: проверять слой доступа к серверу на другой системе значит проверять
# не то, с чем приложение будет работать.
#
# Контейнер НЕ заменяет настоящий сервер целиком. Он закрывает файловые операции,
# выполнение команд, разбор описи и передачу с обрывами. Не закрывает: службы systemd,
# сетевой фильтр, настройку ядра и выдачу сертификата — для них нужен настоящий VPS
# (Фаза 7, см. research.md R-06 и quickstart.md).

FROM ubuntu:24.04

# iproute2 даёт ss: тест мультиплексирования каналов сверяет число соединений
# средствами самого сервера. Без ss эта сверка молча вырождалась в «поверим на слово».
RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      openssh-server \
      ca-certificates \
      iproute2 \
 && rm -rf /var/lib/apt/lists/*

RUN mkdir -p /run/sshd /root/.ssh \
 && chmod 700 /root/.ssh

# Открытая часть того же ключа, которым проверяется разбор ключа с парольной фразой.
# Так интеграционный тест заодно проверяет FR-096 до конца: не «ключ прочитался»,
# а «по нему получилось войти на настоящий OpenSSH».
COPY encrypted_ed25519.key.pub /root/.ssh/authorized_keys
RUN chmod 600 /root/.ssh/authorized_keys

# Пароль нужен, чтобы проверять и второй способ входа, и его отказ.
RUN echo 'root:тестовый-пароль-контейнера' | chpasswd

RUN printf '%s\n' \
      'PermitRootLogin yes' \
      'PasswordAuthentication yes' \
      'PubkeyAuthentication yes' \
      'UsePAM no' \
      'PrintMotd no' \
      > /etc/ssh/sshd_config.d/99-test.conf

# Каталог раздачи в том же месте, что и на настоящем сервере: пути участвуют
# в проверках, и подменять их значило бы проверять другой случай.
RUN mkdir -p /var/lib/vrcast/videos /etc/vrcast

EXPOSE 22
CMD ["/usr/sbin/sshd", "-D", "-e"]
