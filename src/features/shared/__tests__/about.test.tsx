/**
 * T107 — a check that the GPL obligation is kept in fact.
 *
 * What is checked is not that there is a handsome screen, but two promises: that the
 * licence the application is distributed under is named, and that the address where the
 * source of **this very version** can be had is given. A link to "the latest" does not keep
 * the obligation: a person was handed a particular build, and their right is to its source.
 */

import { screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderIn } from "../../../test-utils";

const mockAppVersions = vi.fn();

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  return { ...actual, ipc: { appVersions: () => mockAppVersions() } };
});

const { About } = await import("../About");

beforeEach(() => {
  vi.clearAllMocks();
  mockAppVersions.mockResolvedValue({ app: "0.1.0", server: null, schema: 5 });
});

describe("about the application", () => {
  it("names the licence", async () => {
    renderIn(<About />);
    expect(await screen.findByText(/GNU General Public License/)).toBeInTheDocument();
  });

  it("gives the source address as a link somebody can follow", async () => {
    renderIn(<About />);
    const link = await screen.findByRole("link", { name: /github\.com/ });
    expect(link).toHaveAttribute("href", expect.stringContaining("github.com"));
  });

  it("ties the source to the version in hand", async () => {
    // The GPL obligation is about the build that was received, not about "the latest".
    renderIn(<About />);
    expect(await screen.findByText("v0.1.0")).toBeInTheDocument();
  });

  it("says where the list of other people's work is", async () => {
    renderIn(<About />);
    expect(await screen.findByText("THIRD-PARTY.md")).toBeInTheDocument();
  });

  it("does not go quiet about the licence when the version could not be learned", async () => {
    // The core may be out of reach, and the obligation does not go anywhere because of it.
    mockAppVersions.mockRejectedValue(new Error("the core is unreachable"));
    renderIn(<About />);
    expect(await screen.findByText(/GNU General Public License/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /github\.com/ })).toBeInTheDocument();
  });
});
