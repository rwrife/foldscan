export interface ReviewCapture {
  captureId: string;
  bytes: number;
  sourceIndex: number;
}

export interface RemovedCapture {
  capture: ReviewCapture;
  exportIndex: number;
}

export interface ReviewSession {
  sessionId: string;
  active: ReviewCapture[];
  removed: RemovedCapture[];
}

export interface ReviewPlan {
  sessions: ReviewSession[];
}

export interface ImportSummaryCapture {
  capture_id: string;
  bytes: number;
}

export interface ImportSummarySession {
  session_id: string;
  captures: ImportSummaryCapture[];
}

export interface ImportSummarySource {
  sessions: ImportSummarySession[];
}

function buildSession(source: ImportSummarySession): ReviewSession {
  const active = source.captures.map((capture, index) => ({
    captureId: capture.capture_id,
    bytes: capture.bytes,
    sourceIndex: index,
  }));
  return {
    sessionId: source.session_id,
    active,
    removed: [],
  };
}

export function createReviewPlan(summary: ImportSummarySource): ReviewPlan {
  return {
    sessions: summary.sessions.map(buildSession),
  };
}

function updateSession(
  plan: ReviewPlan,
  sessionId: string,
  mutator: (session: ReviewSession) => ReviewSession,
): ReviewPlan {
  return {
    sessions: plan.sessions.map((session) =>
      session.sessionId === sessionId ? mutator(session) : session,
    ),
  };
}

export function moveCapture(
  plan: ReviewPlan,
  sessionId: string,
  captureId: string,
  delta: -1 | 1,
): ReviewPlan {
  return updateSession(plan, sessionId, (session) => {
    const index = session.active.findIndex((capture) => capture.captureId === captureId);
    if (index < 0) {
      return session;
    }
    const target = index + delta;
    if (target < 0 || target >= session.active.length) {
      return session;
    }
    const active = [...session.active];
    [active[index], active[target]] = [active[target], active[index]];
    return {
      ...session,
      active,
    };
  });
}

export function removeCapture(
  plan: ReviewPlan,
  sessionId: string,
  captureId: string,
): ReviewPlan {
  return updateSession(plan, sessionId, (session) => {
    const index = session.active.findIndex((capture) => capture.captureId === captureId);
    if (index < 0) {
      return session;
    }
    const active = [...session.active];
    const [removedCapture] = active.splice(index, 1);
    const removed = [...session.removed, { capture: removedCapture, exportIndex: index }].sort(
      (a, b) => a.capture.sourceIndex - b.capture.sourceIndex,
    );
    return {
      ...session,
      active,
      removed,
    };
  });
}

export function restoreCapture(
  plan: ReviewPlan,
  sessionId: string,
  captureId: string,
): ReviewPlan {
  return updateSession(plan, sessionId, (session) => {
    const index = session.removed.findIndex(
      (entry) => entry.capture.captureId === captureId,
    );
    if (index < 0) {
      return session;
    }

    const removed = [...session.removed];
    const [entry] = removed.splice(index, 1);
    const active = [...session.active];
    const insertAt = Math.min(entry.exportIndex, active.length);
    active.splice(insertAt, 0, entry.capture);

    return {
      ...session,
      active,
      removed,
    };
  });
}
