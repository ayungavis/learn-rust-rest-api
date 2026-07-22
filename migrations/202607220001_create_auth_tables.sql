CREATE TYPE one_time_token_purpose AS ENUM (
    'email_verification',
    'password_reset'
);

CREATE TABLE users (
    id UUID PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    display_name TEXT NOT NULL,
    email_verified_at TIMESTAMPTZ,
    avatar_object_key TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT users_email_normalized CHECK (email = lower(trim(email))),
    CONSTRAINT users_email_not_empty CHECK (length(email) > 0),
    CONSTRAINT users_password_hash_not_empty CHECK (length(password_hash) > 0),
    CONSTRAINT users_display_name_length CHECK (
        char_length(display_name) BETWEEN 1 AND 100
    )
);

CREATE TABLE sessions (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash BYTEA NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT sessions_token_hash_length CHECK (
        octet_length(token_hash) = 32
    ),
    CONSTRAINT sessions_expiry_after_creation CHECK (
        expires_at > created_at
    ),
    CONSTRAINT sessions_revocation_after_creation CHECK (
        revoked_at IS NULL OR revoked_at >= created_at
    )
);

CREATE INDEX sessions_user_id_idx
ON sessions (user_id);

CREATE INDEX sessions_active_expiry_idx
ON sessions (expires_at)
WHERE revoked_at IS NULL;

CREATE TABLE one_time_tokens (
    id UUID PRIMARY KEY,
    user_ID UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    purpose one_time_token_purpose NOT NULL,
    token_hash BYTEA NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    used_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT one_time_tokens_hash_length CHECK (
        octet_length(token_hash) = 32
    ),
    CONSTRAINT one_time_tokens_expiry_after_creation CHECK (
        expires_at > created_at
    ),
    CONSTRAINT one_time_tokens_used_after_creation CHECK (
        used_at IS NULL OR used_at >= created_at
    )
);

CREATE INDEX one_time_tokens_user_purpose_idx
ON one_time_tokens (user_id, purpose);

CREATE INDEX one_time_tokens_active_expiry_idx
ON one_time_tokens (expires_at)
WHERE used_at IS NULL;
