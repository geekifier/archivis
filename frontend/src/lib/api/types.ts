/**
 * Hand-written API types for auth endpoints.
 *
 * These mirror the Rust backend types and are used when the backend
 * is not running (so openapi-typescript can't generate from the spec).
 * When generated types are available via `npm run generate:api`, prefer those.
 */

export type UserRole = 'admin' | 'user';

export interface User {
	id: string;
	username: string;
	email: string | null;
	role: UserRole;
	is_active: boolean;
	created_at: string;
}

export interface AuthStatusResponse {
	setup_required: boolean;
}

export interface LoginRequest {
	username: string;
	password: string;
}

export interface LoginResponse {
	token: string;
	user: User;
}

export interface SetupRequest {
	username: string;
	password: string;
	email?: string;
}

/** Error envelope returned by the backend for all error responses. */
export interface ApiErrorResponse {
	error: {
		status: number;
		message: string;
	};
}
