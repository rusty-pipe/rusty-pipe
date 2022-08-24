use std::{collections::HashMap};

use anyhow::Error;
use futures::{StreamExt, SinkExt};
use tokio::{io::{AsyncRead, AsyncWrite, AsyncReadExt, split, AsyncWriteExt}, sync::{mpsc, mpsc::{Sender, Receiver}, oneshot}, select};
use tokio_util::codec::{Decoder, Encoder, FramedRead, FramedWrite};
use bytes::{BytesMut, BufMut, Buf};

struct MuxDecoder {}

struct MuxEncoder {}

#[derive(Debug)]
struct MuxFrame {
    stream_id: u8,
    bytes: Vec<u8>
}

impl Decoder for MuxDecoder {
    type Item = MuxFrame;
    type Error = Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 2 {
            return Ok(None);
        }
        let id = src[0];
        let len = src[1];
        if src.len() < usize::from(len) + 2 {
            return Ok(None);
        }
        src.advance(2);
        let bytes = src.split().to_vec();
        return Ok(Some(MuxFrame {stream_id: id, bytes: bytes}));
    }
}

impl Encoder<MuxFrame> for MuxEncoder {
    type Error = Error;

    fn encode(&mut self, item: MuxFrame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        dst.put_u8(item.stream_id);
        dst.put_u8(u8::try_from(item.bytes.len()).unwrap());
        dst.put_slice(&item.bytes);
        return Ok(());
    }
}


// Struct used to multiplex multiple connections over an AsyncReader and AsyncWriter pair
pub struct Multiplexer {
    connection_numbers: [bool; 255],
    connections: HashMap<u8, Sender<MuxFrame>>,
}


impl Multiplexer {

    pub fn new() -> Self {
        Multiplexer { 
            connection_numbers: [false; 255], 
            connections: HashMap::new(),
        }
    }

    fn reserve_id(&mut self) -> Option<u8> {
        for i in 1u8..255 { // numbers 0, 255 are reserved
            if !self.connection_numbers[usize::from(i)] {
                self.connection_numbers[usize::from(i)] = true;
                return Some(i.into());
            }
        }
        return None;
    }

    fn release_id(&mut self, id: u8) {
        self.connection_numbers[usize::from(id)] = false;
    }

    // from connection to out_buffer
    fn pass_outgoing(&mut self, frame_sink: Sender<MuxFrame>, mut stream: impl AsyncRead + Unpin + Send + 'static, id: u8, kill_chan: oneshot::Sender<bool>, kill_sig: oneshot::Receiver<bool>) {
        
        tokio::spawn(async move {
            let mut buf = [0; 255];
            // stream with id 0 means its a new connection
            if let Err(_) = frame_sink.send(MuxFrame{stream_id: 0, bytes: vec![id]}).await {
                return;
            }
            tokio::pin!(kill_sig); 
            loop {
                let len = select! {
                    len = stream.read(&mut buf) => match len {
                        Ok(l) => l,
                        Err(_) => 0
                    },
                    _ = &mut kill_sig => break
                };
                if len < 1 {
                    break;
                }
                let frame = MuxFrame{stream_id: id, bytes: (&buf[..len]).to_vec() };
                log::info!("({})->: {:?}", frame.stream_id, unsafe{std::str::from_utf8_unchecked(&frame.bytes)});
                if let Err(_) = frame_sink.send(frame).await {
                    return;
                }
            }
            // stream with id 255 means its an end to the connection
             _ = frame_sink.send(MuxFrame{stream_id: 255, bytes: vec![id]}).await;
             _ = kill_chan.send(true);
                
        });
        log::info!("New connection: {}", id);
    }

    // from in_buffer to connection
    fn pass_incoming(&mut self, mut frames: Receiver<MuxFrame>, mut sink: impl AsyncWrite + Unpin + Send + 'static, kill_chan: oneshot::Sender<bool>, kill_sig: oneshot::Receiver<bool> ) {
        tokio::spawn(async move {
            tokio::pin!(kill_sig);
            loop {
                let frame = select!{
                    frame = frames.recv() => match frame {
                        Some(frame) => frame,
                        None => break
                    },
                    _ = &mut kill_sig => break
                }; 

                // drop connection
                if frame.stream_id == 255 {
                    
                    break;
                }
                if let Err(_) = sink.write_all(&frame.bytes).await {
                    break;
                }
                log::info!("({})<-: {:?}", frame.stream_id, unsafe{std::str::from_utf8_unchecked(&frame.bytes)});
            }
            _ = kill_chan.send(true);
        });
    }
    // forwards frames to in_buffer and returns frames from out_buffer
    fn pipe_frames(&self, in_buffer: impl AsyncRead + Unpin + Send + 'static, out_buffer: impl AsyncWrite + Unpin + Send + 'static, mut chan: Receiver<MuxFrame>) -> Receiver<MuxFrame> {
       
        let (con_tx, con_rx) = mpsc::channel::<MuxFrame>(1);

        // process outgoing data
        tokio::spawn(async move {
            let encoder = MuxEncoder{};
            let mut out = FramedWrite::new(out_buffer, encoder);
            while let Some(frame) = chan.recv().await {
                if let Err (_) = out.send(frame).await {
                    return;
                }
            }
        });
        
        // process incoming data
        tokio::spawn(async move {
            let decoder = MuxDecoder{};
            let mut framed = FramedRead::new(in_buffer, decoder);
            while let Some(Ok(frame)) = framed.next().await {
                if let Err(_) = con_tx.send(frame).await {
                    return;
                }
            }
        });
        return con_rx;
    }

    fn accept_connection(&mut self, frames: Sender<MuxFrame>, soc: impl AsyncRead + AsyncWrite + Unpin + Send + 'static) {
        let (con_tx, frame_stream) = mpsc::channel::<MuxFrame>(1);
        let (stream, sink) = split(soc);
        let id = match self.reserve_id() {
            Some(id) => id,
            None => {
                log::error!("Reached maximum of 254 connections");
                return;
            }
        };
        let (kill_in, end_in) = oneshot::channel::<bool>();
        let (kill_out, end_out) = oneshot::channel::<bool>();
        self.pass_outgoing(frames, stream, id, kill_in, end_out);
        self.connections.insert(id, con_tx);
        self.pass_incoming( frame_stream, sink, kill_out, end_in);
    }

    fn create_connection(&mut self, frames: Sender<MuxFrame>, id: u8) -> impl AsyncRead + AsyncWrite + Unpin + Send + 'static {
        let (con_tx, con_rx) = mpsc::channel::<MuxFrame>(1);
        
        let (soc_in, soc_out) = tokio::io::duplex(253);
        let (stream, sink) = split(soc_in);
        let (kill_in, end_in) = oneshot::channel::<bool>();
        let (kill_out, end_out) = oneshot::channel::<bool>();
        self.pass_outgoing(frames, stream, id, kill_in, end_out);
        self.connections.insert(id, con_tx);
        self.pass_incoming( con_rx, sink, kill_out, end_in);
        return soc_out;
    }


    fn release_connection(&mut self, id: u8) {
        match self.connections.remove(&id) {
            Some(_) => log::info!("Connection end: {}", id),
            None => {}
        }
        self.release_id(id);
    }

    pub fn consume_connections<T>(mut self, in_buffer: impl AsyncRead + Unpin + Send + 'static, out_buffer: impl AsyncWrite + Unpin + Send + 'static) -> Sender<T>
        where T: AsyncRead + AsyncWrite + Unpin + Send + 'static
    {
        let (con_tx, mut con_rx) = mpsc::channel::<T>(1);
        let (frame_tx, frame_rx) = mpsc::channel::<MuxFrame>(1);
        let (out_frame_tx, mut out_frames) = mpsc::channel::<MuxFrame>(1);
        let mut in_frames = self.pipe_frames(in_buffer, out_buffer, frame_rx);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    // open new connections
                    Some(soc) = con_rx.recv() => self.accept_connection(out_frame_tx.clone(), soc),
                    
                    // from out_buffer to connection
                    Some(frame) = out_frames.recv() => {
                        if frame.stream_id == 255 {
                            self.release_connection(frame.bytes[0]);
                        } 
                        frame_tx.send(frame).await.unwrap();
                        
                        
                    },
                    // from in_buffer to connection
                    Some(frame) = in_frames.recv() => {
                        let frame_id = frame.stream_id;
                        let stream_id = match frame.stream_id {
                            255 => frame.bytes[0],
                            _ => frame.stream_id
                        };
                        match self.connections.get(&stream_id) {
                            Some(con) => {
                                con.send(frame).await.unwrap();
                            },
                            None => {}
                        }
                        if frame_id == 255 { 
                            self.release_connection(stream_id);
                        } 
                        
                    },

                    else => { break } 
                }
            }
        });

        return con_tx;
    }

    pub fn produce_connections(mut self, in_buffer: impl AsyncRead + Unpin + Send + 'static, out_buffer: impl AsyncWrite + Unpin + Send + 'static) -> Receiver<impl AsyncRead + AsyncWrite + Unpin + Send + 'static>
    {
        let (con_tx, con_rx) = mpsc::channel(1);
        let (out_frames_proxy_tx, out_frames_proxy) = mpsc::channel::<MuxFrame>(1);
        let (out_frame_tx, mut out_frames) = mpsc::channel::<MuxFrame>(1);
        let mut in_frames = self.pipe_frames(in_buffer, out_buffer, out_frames_proxy);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    // from out_buffer to connection
                    frame_res = out_frames.recv() => {
                        match frame_res {
                            Some(frame) => {
                                if frame.stream_id == 255 {
                                    self.release_connection(frame.bytes[0]);
                                }
                                out_frames_proxy_tx.send(frame).await.unwrap();
                            },
                            None => {
                                break;
                            }
                        }
                        
                    },
                    
                    //from in_buffer to connection
                    frame_res = in_frames.recv() => {
                        match frame_res {
                            Some(frame) => {
                                let frame_id = frame.stream_id;
                                let stream_id = match frame.stream_id {
                                    255 => frame.bytes[0],
                                    0 => {
                                        let con = self.create_connection(out_frame_tx.clone(), frame.bytes[0]);
                                        if let Err(e) = con_tx.send(con).await {
                                            log::error!("{}", e);
                                        }
                                        continue;
                                    },
                                    _ => frame.stream_id
                                };
                                match self.connections.get(&stream_id) {
                                    Some(con) => {
                                        if let Err(e) = con.send(frame).await {
                                            log::error!("{}", e);
                                        }
                                    },
                                    None => {}
                                }
        
                                if frame_id == 255 {
                                    self.release_connection(stream_id);
                                }
                            },
                            None => {
                                break;
                            }
                        }
                    },
                    else => { break } 
                }
            }
        });

        return con_rx;
    }


}
