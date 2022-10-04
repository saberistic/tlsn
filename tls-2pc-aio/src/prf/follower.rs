use super::{circuits, PRFChannel, PRFError};
use futures::{SinkExt, StreamExt};
use mpc_aio::protocol::{garble::Execute, point_addition::P256SecretShare};
use tls_2pc_core::{
    prf::{self as core, follower_state as state, PRFMessage},
    SessionKeyShares,
};

pub struct MasterSecret {
    core: core::PRFFollower<state::Ms1>,
}

pub struct ClientFinished {
    core: core::PRFFollower<state::Cf1>,
}

pub struct ServerFinished {
    core: core::PRFFollower<state::Sf1>,
}

pub struct PRFFollower<G, S>
where
    G: Execute + Send,
{
    state: S,
    channel: PRFChannel,
    gc_exec: G,
}

impl<G> PRFFollower<G, MasterSecret>
where
    G: Execute + Send,
{
    pub fn new(channel: PRFChannel, gc_exec: G) -> PRFFollower<G, MasterSecret> {
        PRFFollower {
            state: MasterSecret {
                core: core::PRFFollower::new(),
            },
            channel,
            gc_exec,
        }
    }

    pub async fn compute_session_keys(
        mut self,
        secret_share: P256SecretShare,
    ) -> Result<(SessionKeyShares, PRFFollower<G, ClientFinished>), PRFError> {
        let outer_hash_state = circuits::follower_c1(&mut self.gc_exec, secret_share).await?;

        let msg = match self.channel.next().await {
            Some(PRFMessage::LeaderMs1(msg)) => msg,
            Some(m) => return Err(PRFError::UnexpectedMessage(m)),
            None => {
                return Err(PRFError::from(std::io::Error::new(
                    std::io::ErrorKind::ConnectionAborted,
                    "stream closed unexpectedly",
                )))
            }
        };

        let (msg, core) = self.state.core.next(outer_hash_state, msg);

        self.channel.send(PRFMessage::FollowerMs1(msg)).await?;

        let msg = match self.channel.next().await {
            Some(PRFMessage::LeaderMs2(msg)) => msg,
            Some(m) => return Err(PRFError::UnexpectedMessage(m)),
            None => {
                return Err(PRFError::from(std::io::Error::new(
                    std::io::ErrorKind::ConnectionAborted,
                    "stream closed unexpectedly",
                )))
            }
        };

        let (msg, core) = core.next(msg);
        self.channel.send(PRFMessage::FollowerMs2(msg)).await?;

        let msg = match self.channel.next().await {
            Some(PRFMessage::LeaderMs3(msg)) => msg,
            Some(m) => return Err(PRFError::UnexpectedMessage(m)),
            None => {
                return Err(PRFError::from(std::io::Error::new(
                    std::io::ErrorKind::ConnectionAborted,
                    "stream closed unexpectedly",
                )))
            }
        };

        let core = core.next(msg);

        let p2 = core.p2();
        let outer_hash_state =
            circuits::follower_c2(&mut self.gc_exec, outer_hash_state, p2).await?;

        let core = core.next().next(outer_hash_state);

        let msg = match self.channel.next().await {
            Some(PRFMessage::LeaderKe1(msg)) => msg,
            Some(m) => return Err(PRFError::UnexpectedMessage(m)),
            None => {
                return Err(PRFError::from(std::io::Error::new(
                    std::io::ErrorKind::ConnectionAborted,
                    "stream closed unexpectedly",
                )))
            }
        };

        let (msg, core) = core.next(msg);
        self.channel.send(PRFMessage::FollowerKe1(msg)).await?;

        let msg = match self.channel.next().await {
            Some(PRFMessage::LeaderKe2(msg)) => msg,
            Some(m) => return Err(PRFError::UnexpectedMessage(m)),
            None => {
                return Err(PRFError::from(std::io::Error::new(
                    std::io::ErrorKind::ConnectionAborted,
                    "stream closed unexpectedly",
                )))
            }
        };

        let (msg, core) = core.next(msg);
        self.channel.send(PRFMessage::FollowerKe2(msg)).await?;

        let session_keys = circuits::follower_c3(&mut self.gc_exec, outer_hash_state).await?;

        Ok((
            session_keys,
            PRFFollower {
                state: ClientFinished { core },
                channel: self.channel,
                gc_exec: self.gc_exec,
            },
        ))
    }
}

impl<G> PRFFollower<G, ClientFinished>
where
    G: Execute + Send,
{
    /// Computes client finished data using handshake hash
    ///
    /// Returns next state
    pub async fn compute_client_finished(
        mut self,
    ) -> Result<PRFFollower<G, ServerFinished>, PRFError> {
        let msg = match self.channel.next().await {
            Some(PRFMessage::LeaderCf1(msg)) => msg,
            Some(m) => return Err(PRFError::UnexpectedMessage(m)),
            None => {
                return Err(PRFError::from(std::io::Error::new(
                    std::io::ErrorKind::ConnectionAborted,
                    "stream closed unexpectedly",
                )))
            }
        };

        let (msg, core) = self.state.core.next(msg);
        self.channel.send(PRFMessage::FollowerCf1(msg)).await?;

        let msg = match self.channel.next().await {
            Some(PRFMessage::LeaderCf2(msg)) => msg,
            Some(m) => return Err(PRFError::UnexpectedMessage(m)),
            None => {
                return Err(PRFError::from(std::io::Error::new(
                    std::io::ErrorKind::ConnectionAborted,
                    "stream closed unexpectedly",
                )))
            }
        };

        let (msg, core) = core.next(msg);
        self.channel.send(PRFMessage::FollowerCf2(msg)).await?;

        Ok(PRFFollower {
            state: ServerFinished { core },
            channel: self.channel,
            gc_exec: self.gc_exec,
        })
    }
}

impl<G> PRFFollower<G, ServerFinished>
where
    G: Execute + Send,
{
    /// Computes server finished data using handshake hash
    pub async fn compute_server_finished(mut self) -> Result<(), PRFError> {
        let msg = match self.channel.next().await {
            Some(PRFMessage::LeaderSf1(msg)) => msg,
            Some(m) => return Err(PRFError::UnexpectedMessage(m)),
            None => {
                return Err(PRFError::from(std::io::Error::new(
                    std::io::ErrorKind::ConnectionAborted,
                    "stream closed unexpectedly",
                )))
            }
        };

        let (msg, core) = self.state.core.next(msg);
        self.channel.send(PRFMessage::FollowerSf1(msg)).await?;

        let msg = match self.channel.next().await {
            Some(PRFMessage::LeaderSf2(msg)) => msg,
            Some(m) => return Err(PRFError::UnexpectedMessage(m)),
            None => {
                return Err(PRFError::from(std::io::Error::new(
                    std::io::ErrorKind::ConnectionAborted,
                    "stream closed unexpectedly",
                )))
            }
        };

        let msg = core.next(msg);
        self.channel.send(PRFMessage::FollowerSf2(msg)).await?;

        Ok(())
    }
}