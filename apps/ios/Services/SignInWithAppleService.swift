// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements SignInWithAppleService logic in the iOS service layer.
// - Interfaces:
//   - SignInWithAppleError
//   - SignInWithAppleService
// - APIs:
//   - none
//
// Third-party sign-in is disabled. Authentication is email/password only.
#if false
import AuthenticationServices
import Foundation
import UIKit

public enum SignInWithAppleError: Error {
    case missingIdentityToken
    case invalidIdentityTokenEncoding
    case presentationAnchorMissing
    case canceled
    case alreadyInProgress
    case underlying(Error)
}

/// Small async wrapper around ASAuthorizationController to retrieve an Apple identityToken.
public final class SignInWithAppleService: NSObject {
    public static let shared = SignInWithAppleService()

    private var continuation: CheckedContinuation<String, Error>?

    private override init() { super.init() }

    @MainActor
    public func signIn() async throws -> String {
        // Prevent re-entrant sign-in attempts which would overwrite the continuation
        if self.continuation != nil {
            throw SignInWithAppleError.alreadyInProgress
        }

        return try await withCheckedThrowingContinuation { (cont: CheckedContinuation<String, Error>) in
            self.continuation = cont

            let provider = ASAuthorizationAppleIDProvider()
            let request = provider.createRequest()
            request.requestedScopes = []

            let controller = ASAuthorizationController(authorizationRequests: [request])
            controller.delegate = self
            controller.presentationContextProvider = self
            controller.performRequests()
        }
    }
}

extension SignInWithAppleService: ASAuthorizationControllerDelegate {
    public func authorizationController(controller: ASAuthorizationController, didCompleteWithAuthorization authorization: ASAuthorization) {
        guard let credential = authorization.credential as? ASAuthorizationAppleIDCredential else {
            continuation?.resume(throwing: SignInWithAppleError.missingIdentityToken)
            continuation = nil
            return
        }
        guard let tokenData = credential.identityToken else {
            continuation?.resume(throwing: SignInWithAppleError.missingIdentityToken)
            continuation = nil
            return
        }
        guard let token = String(data: tokenData, encoding: .utf8) else {
            continuation?.resume(throwing: SignInWithAppleError.invalidIdentityTokenEncoding)
            continuation = nil
            return
        }

        continuation?.resume(returning: token)
        continuation = nil
    }

    public func authorizationController(controller: ASAuthorizationController, didCompleteWithError error: Error) {
        // Map common cancel case.
        if let ase = error as? ASAuthorizationError, ase.code == .canceled {
            continuation?.resume(throwing: SignInWithAppleError.canceled)
            continuation = nil
            return
        }
        continuation?.resume(throwing: SignInWithAppleError.underlying(error))
        continuation = nil
    }
}

extension SignInWithAppleService: ASAuthorizationControllerPresentationContextProviding {
    public func presentationAnchor(for controller: ASAuthorizationController) -> ASPresentationAnchor {
        // Best-effort: use key window.
        let scenes = UIApplication.shared.connectedScenes
        let windowScene = (scenes.first { $0.activationState == .foregroundActive } as? UIWindowScene)
            ?? scenes.compactMap { $0 as? UIWindowScene }.first
        let window = windowScene?.windows.first { $0.isKeyWindow }
        if let window {
            return window
        }
        if let windowScene {
            return UIWindow(windowScene: windowScene)
        }
        preconditionFailure("No UIWindowScene available for Sign in with Apple presentation")
    }
}
#endif
