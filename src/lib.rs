extern crate time;
extern crate libc;
/*The data types that are to be used by the extern functions. */

use libc::{c_int,uint32_t};

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
}

use std::net::TcpStream;
use std::net::TcpListener;
use std::io::Error;
use std::os::unix::io::AsRawFd;
use time::{get_time, Timespec, Duration};
use std::mem::transmute;



pub struct Poll<'a> {
    fd: c_int,
    streams: Vec<(c_int,TcpStream,&'a mut FnMut(&mut Poll,usize)->c_int)>,
    listeners: Vec<(c_int,TcpListener,&'a mut FnMut(&TcpListener)->c_int)>,
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
        Ok(Poll {fd:fd,streams:Vec::new(),listeners:Vec::new(),timers:Vec::new()})
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

pub fn detach_stream(&mut self,id:i32) -> Result<Result<TcpStream,Error>,()> {
        match self.streams.binary_search_by(|a| a.0.cmp(&id)) {     
            Ok(index) => {
                let (fd,stream,callback) = self.streams.remove(index);
                let no_error;
                unsafe {
                    let mut epoll_event = epoll_event_t {events:0,data:epoll_data_t {fd:0, dummy:0}};
                    no_error = epoll_ctl(self.fd,2,fd,&mut epoll_event);
                }
                if no_error<0 {
                    self.streams.push((fd,stream,callback));
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
        let index =  self.streams.binary_search_by(|a| a.0.cmp(&fd)).unwrap();
        let callback_ptr:* mut FnMut(&mut Poll,usize)->c_int;
        let callback:& mut FnMut(&mut Poll,usize)->c_int;
        unsafe {
        callback_ptr = &mut self.streams[index].2;
        callback = transmute(callback_ptr);
        }
        let new_timeout = callback(self,index);
        self.timers.push((current_time.clone() +Duration::milliseconds(new_timeout as i64),fd));
    }
}

impl<'a> Drop for Poll<'a> {

    fn drop(&mut self) {
        loop {
            match self.streams.pop() {
                None => break,
                Some(s) => { 
                    match self.detach_stream(s.0) {
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
            match self.listeners.pop() {
                None => break,
                Some(_) => () 
            }
        }
        unsafe {
            close(self.fd);
        }
    }
}
