import { beforeEach, describe, expect, it, vi } from "vitest";

import type { EditRequest, EditResponse, ProjectView } from "../api/types";

const backend = vi.hoisted(() => ({
  applyEdit: vi.fn<(request: EditRequest) => Promise<EditResponse>>(),
  chooseClipLutFile: vi.fn<(clipId: number, filePath: string) => Promise<EditResponse>>(),
  undo: vi.fn<() => Promise<ProjectView>>(),
  redo: vi.fn<() => Promise<ProjectView>>(),
}));
const toasts = vi.hoisted(() => ({ show: vi.fn() }));

vi.mock("../api/backend", () => ({ backend }));
vi.mock("./toasts.svelte", () => ({ toasts }));

const { editor } = await import("./editor.svelte");

/** A promise the test settles by hand, standing for an answer still on its way. */
function pendingAnswer<Answer>() {
  let resolve: (answer: Answer) => void = () => undefined;
  let reject: (error: unknown) => void = () => undefined;
  const promise = new Promise<Answer>((onResolve, onReject) => {
    resolve = onResolve;
    reject = onReject;
  });
  return { promise, resolve, reject };
}

function viewAt(revision: number, undoLabel: string | null = null): ProjectView {
  return {
    revision,
    name: "Test",
    filePath: "C:/Test.kadr",
    saveFailure: null,
    settings: { frameRateNumerator: 30, frameRateDenominator: 1 },
    adoptsFirstVideoFormat: false,
    durationTicks: 0,
    media: [],
    tracks: [],
    undoLabel,
    redoLabel: null,
  } as unknown as ProjectView;
}

function answer(view: ProjectView): EditResponse {
  return { view, outcome: { createdClipIds: [], notices: [] } };
}

const ALIGN_LEFT: EditRequest = { type: "setClipText", clipId: 7, changes: { alignment: "left" } };
const NEW_WORDS: EditRequest = { type: "setClipText", clipId: 7, changes: { text: "Bonjour" } };

/** Lets every promise callback already due run. */
async function settle(): Promise<void> {
  for (let turn = 0; turn < 10; turn += 1) {
    await Promise.resolve();
  }
}

describe("editor changes", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    editor.showProject(viewAt(1));
  });

  it("sends an edit only once the one before it is answered", async () => {
    const first = pendingAnswer<EditResponse>();
    const second = pendingAnswer<EditResponse>();
    backend.applyEdit.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);

    const firstEdit = editor.edit(NEW_WORDS);
    const secondEdit = editor.edit(ALIGN_LEFT);
    await settle();
    expect(backend.applyEdit.mock.calls).toEqual([[NEW_WORDS]]);

    first.resolve(answer(viewAt(2)));
    await settle();
    expect(backend.applyEdit.mock.calls).toEqual([[NEW_WORDS], [ALIGN_LEFT]]);
    expect(editor.view?.revision).toBe(2);

    second.resolve(answer(viewAt(3)));
    await Promise.all([firstEdit, secondEdit]);
    expect(editor.view?.revision).toBe(3);
  });

  it("still sends the next edit after a refused one", async () => {
    backend.applyEdit
      .mockRejectedValueOnce({ code: "editRefused", details: "clip 7 does not exist", fileName: null })
      .mockResolvedValueOnce(answer(viewAt(2)));

    const created = await Promise.all([editor.edit(NEW_WORDS), editor.edit(ALIGN_LEFT)]);

    expect(created).toEqual([[], []]);
    expect(backend.applyEdit.mock.calls).toEqual([[NEW_WORDS], [ALIGN_LEFT]]);
    expect(toasts.show).toHaveBeenCalledWith(expect.objectContaining({ tone: "error" }));
    expect(editor.view?.revision).toBe(2);
  });

  it("sends a chosen LUT file only once the edit before it is answered", async () => {
    const strength = pendingAnswer<EditResponse>();
    const LUT_STRENGTH: EditRequest = { type: "setClipLut", clipId: 7, changes: { strength: 0.4 } };
    backend.applyEdit.mockReturnValueOnce(strength.promise);
    backend.chooseClipLutFile.mockResolvedValueOnce(answer(viewAt(3)));

    const changing = editor.edit(LUT_STRENGTH);
    const choosing = editor.chooseLutFile(7, "C:/Looks/teal.cube");
    await settle();
    expect(backend.chooseClipLutFile).not.toHaveBeenCalled();

    strength.resolve(answer(viewAt(2)));
    await Promise.all([changing, choosing]);
    expect(backend.chooseClipLutFile).toHaveBeenCalledWith(7, "C:/Looks/teal.cube");
    expect(editor.view?.revision).toBe(3);
  });

  it("explains an unreadable LUT file and still sends the next edit", async () => {
    backend.chooseClipLutFile.mockRejectedValueOnce({
      code: "lutUnreadable",
      details: "line 4: expected three numbers",
      fileName: "broken.cube",
    });
    backend.applyEdit.mockResolvedValueOnce(answer(viewAt(2)));

    await Promise.all([editor.chooseLutFile(7, "C:/Looks/broken.cube"), editor.edit(ALIGN_LEFT)]);

    expect(toasts.show).toHaveBeenCalledWith(
      expect.objectContaining({ tone: "error", message: expect.stringContaining("broken.cube") }),
    );
    expect(backend.applyEdit.mock.calls).toEqual([[ALIGN_LEFT]]);
    expect(editor.view?.revision).toBe(2);
  });

  it("undoes only after the edit made before it is answered, and names that edit", async () => {
    const edit = pendingAnswer<EditResponse>();
    backend.applyEdit.mockReturnValueOnce(edit.promise);
    backend.undo.mockResolvedValueOnce(viewAt(3));

    const editing = editor.edit(ALIGN_LEFT);
    const undoing = editor.undo();
    await settle();
    expect(backend.undo).not.toHaveBeenCalled();

    edit.resolve(answer(viewAt(2, "changeText")));
    await Promise.all([editing, undoing]);
    expect(backend.undo).toHaveBeenCalledOnce();
    expect(editor.view?.revision).toBe(3);
    expect(toasts.show).toHaveBeenCalledWith(expect.objectContaining({ tone: "info", group: "history" }));
  });
});
