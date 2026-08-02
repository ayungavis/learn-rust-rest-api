CREATE TABLE object_deletion_jobs (
    id UUID PRIMARY KEY,
    object_key TEXT NOT NULL UNIQUE,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    available_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_error TEXT,
    failed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX object_deletion_jobs_pending_idx
    ON object_deletion_jobs (available_at, id)
    WHERE failed_at IS NULL;
