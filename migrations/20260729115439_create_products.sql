CREATE TABLE products (
    id UUID PRIMARY KEY,
    owner_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name VARCHAR(150) NOT NULL,
    description TEXT NOT NULL,
    price_cents BIGINT NOT NULL
        CHECK (price_cents BETWEEN 0 AND 1000000000),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX products_created_at_id_idx
    ON products(created_at, id);

CREATE INDEX products_owner_id_idx
    ON products(owner_id, id);
