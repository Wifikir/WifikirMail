-- Create email jobs table
CREATE TABLE email_jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    recipient VARCHAR(255) NOT NULL,
    template_key VARCHAR(50) NOT NULL,
    reference_id VARCHAR(255) NOT NULL,
    status VARCHAR(50) NOT NULL DEFAULT 'Pending',
    postfix_queue_id VARCHAR(50),
    attempt_count INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    submitted_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    last_error TEXT,
    
    -- Idempotency constraint
    CONSTRAINT unique_reference_template UNIQUE (reference_id, template_key)
);

-- Index for status queries
CREATE INDEX idx_email_jobs_status ON email_jobs(status);
CREATE INDEX idx_email_jobs_postfix_queue_id ON email_jobs(postfix_queue_id) WHERE postfix_queue_id IS NOT NULL;
CREATE INDEX idx_email_jobs_created_at ON email_jobs(created_at);

-- Create status history table
CREATE TABLE email_job_status_history (
    id BIGSERIAL PRIMARY KEY,
    job_id UUID NOT NULL REFERENCES email_jobs(id) ON DELETE CASCADE,
    status VARCHAR(50) NOT NULL,
    changed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    error_message TEXT,
    metadata JSONB
);

CREATE INDEX idx_status_history_job_id ON email_job_status_history(job_id);
CREATE INDEX idx_status_history_changed_at ON email_job_status_history(changed_at);

-- Create API keys table
CREATE TABLE api_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    key_hash VARCHAR(64) NOT NULL UNIQUE,
    name VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_used_at TIMESTAMPTZ,
    is_active BOOLEAN NOT NULL DEFAULT true
);

CREATE INDEX idx_api_keys_key_hash ON api_keys(key_hash) WHERE is_active = true;
