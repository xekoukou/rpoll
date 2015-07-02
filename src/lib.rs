extern crate time;
extern crate libc;
/*The data types that are to be used by the extern functions. */

use libc::{c_void,ssize_t,c_int,c_uint,uint32_t,size_t};

#[repr(C)]
struct epoll_data_t {
fd: c_int,
// Is this necessary?
dummy: c_int
}

#[repr(C)]
struct epoll_event_t {
    events: uint32_t,
    data: epoll_data_t
}
extern {

fn epoll_create1(flags: c_int) -> c_int;
fn epoll_ctl(epfd:c_int,op:c_int,fd:c_int,epoll_event:*mut epoll_event_t) ->c_int;
fn epoll_wait(epfd: c_int,events:*mut epoll_event_t,maxevents: c_int, timeout: c_int) ->c_int;
fn close(fd:c_int) -> c_int;
fn eventfd(initval:c_uint,flags:c_int) -> c_int;
fn write(fd:c_int,buf: *const c_void, count:size_t) -> ssize_t;
}

use std::net::TcpStream;
use std::io::Error;
use std::os::unix::io::AsRawFd;
use time::{get_time, Timespec, Duration};
use std::mem::{size_of,transmute};
use std::sync::mpsc;
use std::sync::mpsc::{SendError,Receiver,Sender};

pub type PReceiver<T> = Receiver<T>;
pub struct PSender<T> {
    sender:Sender<T>,
    fd: c_int
}


impl<T> PSender<T> {

   fn send(&self, t:T) -> Result<(),SendError<T>> {
        self.sender.send(t)
    }

}

//TODO These functions can be misused by the programer by specifying the wrong Type.
/// It is the programers responcibity to provide the same type to both send and receive
pub fn send<T:Sized>(sender:&PSender<i8>,t:T) ->Result<(),SendError<i8>> {
    let ba = &t as *const _ as *const i8;
    let mut result=Ok(());
    for i in 0..size_of::<T>()-1 {
            //TODO Handle Errors
        unsafe {
            result = sender.send(*ba.offset(i as isize));
        }
        match result {
            Ok(()) => {},
            Err(_) => {
                break;
            }
        }
    }
    match result {
        Ok(()) => unsafe {
            let buf:u64 = 0;
            //TODO Do not crash the application. Return an error instead.
            let nwritten = write(sender.fd,&buf as *const _ as *const c_void,8);
            assert!(nwritten == 8);
            Ok(())
        },
        Err(send_error) => {
            Err(send_error)
        }
    }
}
pub fn receive<T:Sized>(receiver:&PReceiver<i8>) ->T {
    let mut t:T;
    unsafe {
     t = std::mem::uninitialized();
    }
    for i in 0..size_of::<T>()-1 {
        unsafe {
            let ba = (&mut t as *mut _ as *mut i8).offset(i as isize);
            //TODO Handle Errors
            *ba = receiver.recv().unwrap();
        }
    }
    t
}


fn pchannel<T>() -> (PSender<T>,PReceiver<T>) {
    let fd;
    unsafe {
      fd = eventfd(0,0);
    }
    let (sender,receiver) = mpsc::channel();

    (PSender { sender: sender, fd: fd } , receiver )
}

pub struct Poll<'a> {
    fd: c_int,
    streams: Vec<(c_int,TcpStream,&'a mut FnMut(&mut Poll,usize)->c_int)>,
    channels: Vec<(c_int,PReceiver<i8>,&'a mut FnMut(&mut Poll,usize)->c_int)>,
    timers: Vec<(Timespec,c_int)>
}

pub enum PollEvent {
    EPOLLIN = 0x001,
    EPOLLOUT = 0x004
}


impl<'a> Poll<'a> {

pub fn new() -> Result<Self,Error> {
        let fd;
        unsafe {
            fd = epoll_create1(0);
        }
        if fd<0 {
            Err(std::io::Error::last_os_error())
        } else {
        Ok(Poll {fd:fd,streams:Vec::new(),channels:Vec::new(),timers:Vec::new()})
        }
    }

pub fn attach_stream(&mut self,stream:TcpStream,event:PollEvent,callback: &'a mut FnMut(&mut Poll,usize)->i32) -> Result<Result<i32,Error>,()> {
       let fd = stream.as_raw_fd();
       match self.streams.binary_search_by(|a| a.0.cmp(&fd)) {
           Ok(_) => Err(()),
           Err(_) => {
       self.streams.push((fd,stream,callback));
       let no_error;
       unsafe {
           let mut epoll_event = epoll_event_t {events:event as uint32_t,data:epoll_data_t {fd:fd, dummy:0}};
           no_error = epoll_ctl(self.fd,1,fd, &mut epoll_event);
       }
       if no_error<0 {
           Ok(Err(std::io::Error::last_os_error()))
       } else {
           self.streams.sort_by(|a,b| a.0.cmp(&b.0));
           Ok(Ok(fd))
       }
       }
       } 
    }

pub fn detach_stream(&mut self,fd:i32) -> Result<Result<TcpStream,Error>,()> {
        match self.streams.binary_search_by(|a| a.0.cmp(&fd)) {     
            Ok(index) => {
                let (_,stream,callback) = self.streams.remove(index);
                let no_error;
                unsafe {
                    let mut epoll_event = epoll_event_t {events:0,data:epoll_data_t {fd:0, dummy:0}};
                    no_error = epoll_ctl(self.fd,2,fd,&mut epoll_event);
                }
                if no_error<0 {
                    self.streams.push((fd,stream,callback));
                    self.streams.sort_by(|a,b| a.0.cmp(&b.0));
                    Ok(Err(std::io::Error::last_os_error()))
                } else {
                    Ok(Ok(stream))
                }
            },
           Err(_) => {
               Err(())
           }            
        }
    }

pub fn borrow_stream(&self, index:usize) ->&TcpStream {
       &self.streams[index].1
    }


pub fn wait(&mut self,timeout:i32) ->Result<(),Error> {
        let mut events:[epoll_event_t;200];
        let mut number_of_events;
        unsafe {
            events = std::mem::uninitialized();
            number_of_events = epoll_wait(self.fd,&mut events[0] as *mut epoll_event_t,200,timeout);
        }
        if number_of_events < 0 {
            Err(std::io::Error::last_os_error())
        } else {
        let current_time = get_time();
        while !self.timers.is_empty() {
            if self.timers[0].0.lt(&current_time) {
                let fd = self.timers[0].1;
                self.perform_callback(fd,&current_time);
                self.timers.pop();
            } else {
                break;
            } 
        }
        for i in 0..number_of_events {
            let fd = events[i as usize].data.fd;
            self.perform_callback(fd,&current_time);
        }
        self.timers.sort_by(|a,b| a.0.cmp(&b.0));
        if self.timers.is_empty() {
            self.wait(-1)
        } else {
            let mut new_timeout = (self.timers[0].0-get_time()).num_milliseconds() as c_int;
            if new_timeout < 0 {
                new_timeout = 0;
            }
            self.wait(new_timeout)
        }
        }
    }

    fn perform_callback(&mut self,fd:c_int,current_time:&Timespec) {
        match self.streams.binary_search_by(|a| a.0.cmp(&fd)) {
            Ok(index) => {
            let callback_ptr:* mut FnMut(&mut Poll,usize)->c_int;
            let callback:& mut FnMut(&mut Poll,usize)->c_int;
            unsafe {
                callback_ptr = &mut self.streams[index].2;
                callback = transmute(callback_ptr);
            }
            let new_timeout = callback(self,index);
            if new_timeout >= 0 {
                self.timers.push((current_time.clone() +Duration::milliseconds(new_timeout as i64),fd));
            }},
            Err(_) => {
            let index = self.channels.binary_search_by(|a| a.0.cmp(&fd)).unwrap();
            let callback_ptr:* mut FnMut(&mut Poll,usize)->c_int;
            let callback:& mut FnMut(&mut Poll,usize)->c_int;
            unsafe {
                callback_ptr = &mut self.channels[index].2;
                callback = transmute(callback_ptr);
            }
            let new_timeout = callback(self,index);
            if new_timeout >= 0 {
                self.timers.push((current_time.clone() +Duration::milliseconds(new_timeout as i64),fd));
            }}
        }
    }

    pub fn create_channel(&mut self, callback: &'a mut FnMut(&mut Poll,usize)->c_int) -> Result<PSender<i8>,Error> {
        let (sender,receiver) = pchannel();
        self.channels.push((sender.fd,receiver, callback));
        let no_error;
        unsafe {
            let mut epoll_event = epoll_event_t {events:0x001,data:epoll_data_t {fd:sender.fd, dummy:0}};
            no_error = epoll_ctl(self.fd,1,sender.fd, &mut epoll_event);
        }
        if no_error<0 {
            Err(std::io::Error::last_os_error())
        } else {
            self.channels.sort_by(|a,b| a.0.cmp(&b.0));
            Ok(sender)
        }
    }

    pub fn destroy_channel(&mut self, sender:PSender<i8>) ->Result<Result<(),Error>,()> {
        self.destroy_channel_by_fd(sender.fd)
    }
    fn destroy_channel_by_fd(&mut self, fd:c_int) ->Result<Result<(),Error>,()> {
        match self.channels.binary_search_by(|a| a.0.cmp(&fd)) {     
            Ok(index) => {
                let (_,receiver,callback) = self.channels.remove(index);
                let no_error;
                unsafe {
                    let mut epoll_event = epoll_event_t {events:0,data:epoll_data_t {fd:0, dummy:0}};
                    no_error = epoll_ctl(self.fd,2,fd,&mut epoll_event);
                }
                if no_error<0 {
                    self.channels.push((fd,receiver,callback));
                    self.channels.sort_by(|a,b| a.0.cmp(&b.0));
                    Ok(Err(std::io::Error::last_os_error()))
                } else {
                    Ok(Ok(()))
                }
            },
           Err(_) => {
               Err(())
           }            
        }
    }


    pub fn borrow_channel(&self, index:usize) ->&PReceiver<i8> {
       &self.channels[index].1
    }

}

impl<'a> Drop for Poll<'a> {

    fn drop(&mut self) {
        loop {
            let rfd:Option<c_int>;
            match self.streams.last() {
                None => rfd = None,
                Some(s) => rfd = Some(s.0)
            };
            match rfd {
                None => break,
                Some(fd) => { 
                    match self.detach_stream(fd) {
                        Ok(_) => (),
                        Err(_) => {
                            println!("Error: Poll Object could't be destroyed correctly");
                            std::process::exit(1);
                        }
                    }
                }
            }
        }
        loop {
            let rfd:Option<c_int>;
            match self.channels.last() {
                None => rfd = None,
                Some(s) => rfd = Some(s.0)
            };
            match rfd {
                None => break,
                Some(fd) => { 
                    match self.destroy_channel_by_fd(fd) {
                        Ok(_) => (),
                        Err(_) => {
                            println!("Error: Poll Object could't be destroyed correctly");
                            std::process::exit(1);
                        }
                    }
                }
            }
        }
        unsafe {
            close(self.fd);
        }
    }
}
