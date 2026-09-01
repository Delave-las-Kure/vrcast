/**
 * T352 — the update screen (FR-113).
 *
 * **The check that earns this file is the first one: opening the screen asks nobody anything.**
 * The rule — never reach for the network unbidden — is kept by there being two calls, one that
 * cannot leave the machine and one that does; a test that only read the screen's text would
 * pass just as happily on a version that checked on mount and said nothing about it.
 *
 * The rest is the difference between three copies of the same application: a package that will
 * ask for an administrator password, an AppImage that will not, and a build from the source
 * tree that cannot update at all. One careful sentence covering all three would be true and
 * useless.
 */

import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { renderIn, ru } from "../../../test-utils";
import type { Found, TaskOnClose, UpdateStanding } from "../../../shared/contract";

const shared = vi.hoisted(() => ({
  standing: vi.fn(),
  check: vi.fn(),
  install: vi.fn(),
  onClose: vi.fn(),
}));

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  // Built from the real `ipc` rather than listed by hand (T470). Imported here
  // because `vi.mock` is hoisted above every import in the file.
  const { stubIpc } = await import("../../../test-ipc");
  return {
    ...actual,
    ipc: stubIpc(actual.ipc as unknown as Record<string, unknown>, {
      updateStanding: () => shared.standing(),
      updateCheck: () => shared.check(),
      updateInstall: (confirmed: boolean) => shared.install(confirmed),
      tasksOnClose: () => shared.onClose(),
    }),
  };
});

const { Update } = await import("../Update");

const INSTALLED: UpdateStanding = {
  current: "1.2.3",
  installed_as: "windows",
  configured: true,
};

const NEWER: Found = {
  kind: "available",
  version: "1.3.0",
  notes: "Works the ladder out faster",
  date: "2026-08-28",
};

const BUSY: TaskOnClose[] = [
  {
    id: "t1",
    kind: "convert",
    progress: 0.4,
    outcome: "restarts",
    explanation: { key: "ON_CLOSE_RESTARTS_LOSING", params: { percent: 40 } },
  },
];

beforeEach(() => {
  vi.clearAllMocks();
  shared.standing.mockResolvedValue(INSTALLED);
  shared.check.mockResolvedValue({ kind: "up_to_date" } as Found);
  shared.install.mockResolvedValue(undefined);
  shared.onClose.mockResolvedValue([]);
});

describe("the update screen", () => {
  it("asks nobody anything until it is asked to", async () => {
    renderIn(<Update />);
    // The version appears, so the screen has finished what it does on opening.
    await screen.findByText("1.2.3");
    expect(shared.standing).toHaveBeenCalled();
    expect(shared.check).not.toHaveBeenCalled();
  });

  it("checks when the button is pressed, and not before", async () => {
    renderIn(<Update />);
    await screen.findByText("1.2.3");

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: ru.ui.update.check }));
    });
    await waitFor(() => expect(shared.check).toHaveBeenCalledTimes(1));
    expect(await screen.findByText(ru.ui.update.upToDate)).toBeInTheDocument();
  });

  it("says a build without update settings has nowhere to look, and offers no button", async () => {
    shared.standing.mockResolvedValue({ ...INSTALLED, configured: false });
    renderIn(<Update />);

    expect(await screen.findByText(ru.ui.update.notConfigured)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: ru.ui.update.check })).not.toBeInTheDocument();
  });

  it("says a build from the source tree has nothing to update", async () => {
    shared.standing.mockResolvedValue({ ...INSTALLED, installed_as: "unpackaged" });
    renderIn(<Update />);

    expect(await screen.findByText(ru.ui.update.unpackaged)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: ru.ui.update.check })).not.toBeInTheDocument();
  });

  it("shows what is running before offering to install, because installing stops it", async () => {
    shared.check.mockResolvedValue(NEWER);
    shared.onClose.mockResolvedValue(BUSY);
    renderIn(<Update />);
    await screen.findByText("1.2.3");

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: ru.ui.update.check }));
    });

    expect(await screen.findByText(ru.ui.update.available("1.3.0"))).toBeInTheDocument();
    await waitFor(() => expect(shared.onClose).toHaveBeenCalled());
    // The same wording the close dialog uses — one answer to one question.
    expect(screen.getByText(ru.ui.tasks.closeLosing)).toBeInTheDocument();
  });

  it("does not frighten a Linux copy about tasks nothing is going to stop", async () => {
    // Only the Windows installer stops the application. On Linux the plugin rewrites the
    // AppImage or hands the package to `dpkg`, and the running copy carries on with the old
    // code — so a list of endangered tasks there warns about something that will not happen.
    shared.check.mockResolvedValue(NEWER);
    shared.onClose.mockResolvedValue(BUSY);
    shared.standing.mockResolvedValue({ ...INSTALLED, installed_as: "deb" });
    renderIn(<Update />);
    await screen.findByText("1.2.3");

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: ru.ui.update.check }));
    });

    await screen.findByText(ru.ui.update.available("1.3.0"));
    expect(screen.queryByText(ru.ui.tasks.closeLosing)).not.toBeInTheDocument();
    expect(shared.onClose).not.toHaveBeenCalled();
  });

  it("says the new version starts next time, where the old one keeps running", async () => {
    // Without this the button simply goes quiet and nothing appears to have happened.
    shared.check.mockResolvedValue(NEWER);
    shared.standing.mockResolvedValue({ ...INSTALLED, installed_as: "app_image" });
    renderIn(<Update />);
    await screen.findByText("1.2.3");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: ru.ui.update.check }));
    });
    await act(async () => {
      fireEvent.click(screen.getByLabelText(ru.ui.update.agree));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: ru.ui.update.install }));
    });
    expect(await screen.findByTestId("update-installed")).toBeInTheDocument();
  });

  it("warns a package copy about the password, and an AppImage not at all", async () => {
    shared.check.mockResolvedValue(NEWER);
    shared.standing.mockResolvedValue({ ...INSTALLED, installed_as: "deb" });
    const { unmount } = renderIn(<Update />);
    await screen.findByText("1.2.3");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: ru.ui.update.check }));
    });
    expect(await screen.findByText(ru.ui.update.warnPackage)).toBeInTheDocument();
    unmount();

    shared.standing.mockResolvedValue({ ...INSTALLED, installed_as: "app_image" });
    renderIn(<Update />);
    await screen.findByText("1.2.3");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: ru.ui.update.check }));
    });
    expect(await screen.findByText(ru.ui.update.warnAppImage)).toBeInTheDocument();
    expect(screen.queryByText(ru.ui.update.warnPackage)).not.toBeInTheDocument();
  });

  it("will not install until the consequence has been agreed to", async () => {
    shared.check.mockResolvedValue(NEWER);
    renderIn(<Update />);
    await screen.findByText("1.2.3");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: ru.ui.update.check }));
    });

    const install = await screen.findByRole("button", { name: ru.ui.update.install });
    expect(install).toBeDisabled();

    await act(async () => {
      fireEvent.click(screen.getByLabelText(ru.ui.update.agree));
    });
    expect(install).toBeEnabled();
    await act(async () => {
      fireEvent.click(install);
    });
    expect(shared.install).toHaveBeenCalledWith(true);
  });
});
