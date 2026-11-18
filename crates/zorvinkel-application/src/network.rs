use crate::state::{ApplicationError, StateStore};
use zorvinkel_core::{network::*, registry::ClientKey};

/// Adapters must bound every system call and return only observed configurations.
pub trait NetworkControl {
    fn check_support(&mut self, intent: &NetworkIntent) -> Result<(), NetworkError>;
    fn observe(&mut self) -> Result<NetworkConfiguration, NetworkError>;
    fn apply(&mut self, intent: &NetworkIntent) -> Result<(), NetworkError>;
    fn restore(&mut self, previous: &NetworkConfiguration) -> Result<(), NetworkError>;
}

pub fn set_restart_policy(
    store: &mut impl StateStore,
    actor: &ClientKey,
    policy: RestartPolicy,
) -> Result<(), ApplicationError> {
    store.update(|state| {
        state.registry.authorize(actor)?;
        state.settings.restart_policy = policy;
        Ok(())
    })
}

/// A retry with an existing ID only returns its journal entry, never repeats effects.
pub fn execute(
    store: &mut impl StateStore,
    network: &mut impl NetworkControl,
    actor: &ClientKey,
    id: OperationId,
    intent: NetworkIntent,
) -> Result<NetworkOperation, ApplicationError> {
    let current = store.read()?;
    current.registry.authorize(actor)?;
    if let Some(operation) = current.operations.get(&id) {
        return matching_operation(operation, actor, &intent);
    }
    network.check_support(&intent)?;
    let (operation, claimed) = store.update(|state| {
        state.registry.authorize(actor)?;
        if let Some(operation) = state.operations.get(&id) {
            return Ok((matching_operation(operation, actor, &intent)?, false));
        }
        if state
            .operations
            .values()
            .any(|operation| operation.status.is_pending())
        {
            return Err(ApplicationError::Busy);
        }
        if state.operations.len() >= 256 {
            return Err(ApplicationError::HistoryFull);
        }
        let previous = network.observe()?;
        let operation = NetworkOperation {
            id,
            owner: actor.clone(),
            intent: intent.clone(),
            previous,
            status: OperationStatus::Pending,
        };
        state.operations.insert(id, operation.clone());
        Ok((operation, true))
    })?;
    if !claimed {
        return Ok(operation);
    }
    let result = network.apply(&intent).and_then(|()| {
        let actual = network.observe()?;
        if actual == intent.target(&operation.previous) {
            Ok(actual)
        } else {
            Err(NetworkError::Rejected)
        }
    });
    let status = match result {
        Ok(actual) => OperationStatus::Succeeded { actual },
        Err(cause) => {
            let restore_failed = network.restore(&operation.previous).is_err();
            match network.observe() {
                Ok(actual) => {
                    let rollback_failed = restore_failed || actual != operation.previous;
                    OperationStatus::Failed {
                        cause,
                        actual,
                        rollback_failed,
                    }
                }
                Err(_) => OperationStatus::NeedsReconciliation { cause },
            }
        }
    };
    store.update(|state| {
        let operation = state
            .operations
            .get_mut(&id)
            .ok_or(ApplicationError::Conflict)?;
        if operation.status != OperationStatus::Pending {
            return Err(ApplicationError::Conflict);
        }
        if let OperationStatus::Succeeded { actual } = &status {
            state.settings.last_accepted = actual.clone();
        }
        operation.status = status;
        Ok(operation.clone())
    })
}

fn matching_operation(
    operation: &NetworkOperation,
    actor: &ClientKey,
    intent: &NetworkIntent,
) -> Result<NetworkOperation, ApplicationError> {
    if &operation.owner != actor || &operation.intent != intent {
        return Err(ApplicationError::Conflict);
    }
    Ok(operation.clone())
}

/// Run exclusively during startup, before accepting mutations. Never repeats an intent
/// or promotes a pending configuration to an accepted restart configuration.
pub fn reconcile(
    store: &mut impl StateStore,
    network: &mut impl NetworkControl,
) -> Result<(), ApplicationError> {
    let actual = network.observe()?;
    store.update(|state| {
        for operation in state
            .operations
            .values_mut()
            .filter(|operation| operation.status.is_pending())
        {
            operation.status = OperationStatus::Failed {
                cause: NetworkError::Interrupted,
                actual: actual.clone(),
                rollback_failed: actual != operation.previous,
            };
        }
        Ok(())
    })
}
